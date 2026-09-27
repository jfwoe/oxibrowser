//! Origin policy for credential use — the phishing defense that decides
//! *where* an agent's stored credentials may be injected.
//!
//! Mirrors [`IpFilter`]'s role for SSRF: a deterministic, deny-first gate.
//! Matching is **exact origin** (scheme + host + port, normalized); a
//! registrable-domain match is reportable but never authorizes automatic
//! credential use (Apple password-manager-resources and Chromium treat
//! suffix matches as suggestion-only, see design §9 FM-2). Host comparisons
//! respect DNS label boundaries — `evil-example.com` and `notexample.com`
//! never match `example.com`.
//!
//! Consumers arrive with the credential broker (P1-M2/M4); this module
//! ships the policy primitives and their tests first.

/// A normalized web origin: lowercase scheme, lowercased (IDNA→punycode)
/// host, effective port (default ports removed).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Origin {
    scheme: String,
    host: String,
    port: u16,
}

/// Origin parse failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid origin: {0}")]
pub struct OriginError(pub String);

impl Origin {
    /// Parse and normalize an absolute origin URL string
    /// (`https://dash.cloudflare.com` or with explicit port). Relative or
    /// non-hierarchical inputs are rejected.
    pub fn parse(input: &str) -> Result<Self, OriginError> {
        let url = url::Url::parse(input).map_err(|_| OriginError(input.to_string()))?;
        if !url.has_host() {
            return Err(OriginError(input.to_string()));
        }
        let scheme = url.scheme().to_ascii_lowercase();
        if scheme != "http" && scheme != "https" {
            return Err(OriginError(format!("unsupported scheme: {scheme}")));
        }
        let host = url
            .host_str()
            .map(|h| h.to_ascii_lowercase())
            .ok_or_else(|| OriginError(input.to_string()))?;
        // `url` already applies IDNA (punycode) to hosts and `port_or_known_default`
        // removes the default port.
        let port = url
            .port_or_known_default()
            .ok_or_else(|| OriginError(format!("no port resolvable for scheme {scheme}")))?;
        Ok(Self { scheme, host, port })
    }

    /// Exact comparison against another normalized origin.
    pub fn exact_eq(&self, other: &Origin) -> bool {
        self == other
    }

    /// DNS label-boundary check: this origin's host equals `registrable` or
    /// is a subdomain of it (`host == key || host.ends_with("." + key)`).
    /// Plain suffix comparison is deliberately impossible to express here —
    /// `host.ends_with("example.com")` would admit `notexample.com`.
    pub fn host_within_registrable(&self, registrable: &str) -> bool {
        let key = registrable
            .trim()
            .trim_end_matches('.')
            .to_ascii_lowercase();
        if key.is_empty() {
            return false;
        }
        self.host == key || self.host.ends_with(&format!(".{key}"))
    }

    /// Serialize back to `scheme://host[:port]` (port omitted when default).
    pub fn as_str(&self) -> String {
        let default = match self.scheme.as_str() {
            "https" => 443,
            _ => 80,
        };
        if self.port == default {
            format!("{}://{}", self.scheme, self.host)
        } else {
            format!("{}://{}:{}", self.scheme, self.host, self.port)
        }
    }

    /// The host component (punycode, lowercase).
    pub fn host(&self) -> &str {
        &self.host
    }
}

impl std::fmt::Display for Origin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.as_str())
    }
}

/// Matching strength tier for a candidate origin against an allowlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OriginMatch {
    /// Same scheme, host, and effective port — the only tier that may
    /// authorize automatic credential use.
    Exact,
    /// Different host within the same registrable domain (e.g. a sibling or
    /// apex/subdomain pair). Suggestion-only; never auto-authorized.
    RegistrableDomain,
    /// No relationship.
    None,
}

/// Policy evaluation outcome. `Deny` always wins over any allowance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    RequireConfirmation { reason: String },
    Deny { reason: String },
}

/// Rule mode for static policy entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleMode {
    Allow,
    Deny,
}

/// A static origin rule consulted before consent records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OriginRule {
    pub origin: Origin,
    pub mode: RuleMode,
}

/// Verdict for a navigation observed while a credential session is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedirectVerdict {
    /// The new origin stays within the agreed origin set.
    Continue,
    /// Unload the credential, audit a `policy_violation`, and request
    /// re-escalation before any further credential use.
    InvalidateAndEscalate,
}

/// Deny-first origin policy for credential use.
///
/// Evaluation order mirrors the Claude Code IAM shape the research adopted
/// (design §2/§6): static deny rules → explicit allow rules → consent
/// cache → confirmation. Consent handling lives in the broker's
/// `PolicyEngine`; this type owns the origin math.
#[derive(Debug, Clone, Default)]
pub struct OriginPolicy {
    rules: Vec<OriginRule>,
}

impl OriginPolicy {
    pub fn from_rules(rules: Vec<OriginRule>) -> Self {
        Self { rules }
    }

    /// Evaluate a credential-use request.
    ///
    /// * `allowed` — the credential record's exact-origin allowlist.
    /// * `top_level` — the page the agent believes it is on.
    /// * `frame` — the origin of the frame that actually hosts the form.
    ///   `None` (unknown frame origin) is **fail-closed**: a top-level-only
    ///   check would admit cross-origin iframe form injection.
    pub fn evaluate(
        &self,
        allowed: &[Origin],
        top_level: &Origin,
        frame: Option<&Origin>,
    ) -> Decision {
        // 1. Static deny rules win over everything.
        for rule in &self.rules {
            if rule.mode == RuleMode::Deny
                && (rule.origin.exact_eq(top_level)
                    || frame.map(|f| rule.origin.exact_eq(f)).unwrap_or(false))
            {
                return Decision::Deny {
                    reason: format!("deny rule for {}", rule.origin),
                };
            }
        }
        // 2. Frame origin must be known and exact-matched.
        let Some(frame_origin) = frame else {
            return Decision::Deny {
                reason: "frame origin unknown — refusing credential use".to_string(),
            };
        };
        // 3. Both the page and the frame must be in the exact allowlist.
        let top_allowed = allowed.iter().any(|o| o.exact_eq(top_level));
        let frame_allowed = allowed.iter().any(|o| o.exact_eq(frame_origin));
        match (top_allowed, frame_allowed) {
            (true, true) => Decision::Allow,
            (false, _) | (_, false) => Decision::Deny {
                reason: format!(
                    "origin not in credential allowlist (top={top_level}, frame={frame_origin})"
                ),
            },
        }
    }

    /// Classify a candidate origin against the allowlist tier.
    pub fn classify(&self, allowed: &[Origin], candidate: &Origin) -> OriginMatch {
        if allowed.iter().any(|o| o.exact_eq(candidate)) {
            return OriginMatch::Exact;
        }
        // Registrable-domain relationship: every allowlist origin whose
        // registrable domain contains the candidate host, or vice versa.
        for o in allowed {
            if candidate.host_within_registrable(&registrable_of(o))
                || o.host_within_registrable(&registrable_of(candidate))
            {
                return OriginMatch::RegistrableDomain;
            }
        }
        OriginMatch::None
    }

    /// Redirect verdict while a credential session is active.
    pub fn redirect_verdict(&self, allowed: &[Origin], next: &Origin) -> RedirectVerdict {
        if allowed.iter().any(|o| o.exact_eq(next)) {
            RedirectVerdict::Continue
        } else {
            RedirectVerdict::InvalidateAndEscalate
        }
    }
}

/// Registrable domain of an origin's host via the public suffix list; falls
/// back to the full host when the PSL has no entry (single-label hosts,
/// `localhost`).
fn registrable_of(origin: &Origin) -> String {
    psl::domain_str(origin.host())
        .map(|d| d.to_string())
        .unwrap_or_else(|| origin.host().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn o(s: &str) -> Origin {
        Origin::parse(s).unwrap()
    }

    #[test]
    fn parses_and_normalizes() {
        assert_eq!(
            o("https://DASH.Example.com").as_str(),
            "https://dash.example.com"
        );
        assert_eq!(o("https://example.com:443").as_str(), "https://example.com");
        assert_eq!(o("http://example.com:80").as_str(), "http://example.com");
        assert_eq!(
            o("http://example.com:8080").as_str(),
            "http://example.com:8080"
        );
        // Unicode host normalizes to punycode via the url crate.
        let uni = Origin::parse("https://예시.com").unwrap();
        assert!(uni.host().starts_with("xn--"), "{}", uni.host());
    }

    #[test]
    fn rejects_bad_origins() {
        assert!(Origin::parse("not a url").is_err());
        assert!(Origin::parse("ftp://example.com").is_err());
        assert!(Origin::parse("mailto:a@b.c").is_err());
    }

    #[test]
    fn scheme_and_port_are_part_of_identity() {
        assert!(!o("https://example.com").exact_eq(&o("http://example.com")));
        assert!(!o("https://example.com").exact_eq(&o("https://example.com:8443")));
        assert!(o("https://example.com").exact_eq(&o("HTTPS://EXAMPLE.COM")));
    }

    #[test]
    fn dns_label_boundary_rejects_lookalikes() {
        let origin = o("https://notexample.com");
        assert!(!origin.host_within_registrable("example.com"));
        let evil = o("https://evil-example.com");
        assert!(!evil.host_within_registrable("example.com"));
        // Legit subdomains do match, with dot-boundary semantics.
        assert!(o("https://app.example.com").host_within_registrable("example.com"));
        assert!(o("https://example.com").host_within_registrable("example.com"));
        assert!(!o("https://example.org").host_within_registrable("example.com"));
    }

    #[test]
    fn evaluate_requires_exact_match_on_top_and_frame() {
        let allowed = vec![
            o("https://dash.cloudflare.com"),
            o("https://login.cloudflare.com"),
        ];
        let policy = OriginPolicy::default();

        assert_eq!(
            policy.evaluate(
                &allowed,
                &o("https://dash.cloudflare.com"),
                Some(&o("https://dash.cloudflare.com"))
            ),
            Decision::Allow
        );
        // Sibling subdomain of the same registrable domain: deny.
        assert!(matches!(
            policy.evaluate(
                &allowed,
                &o("https://evil.cloudflare.com.evil.net"),
                Some(&o("https://evil.cloudflare.com.evil.net"))
            ),
            Decision::Deny { .. }
        ));
        // Different scheme = different origin = deny.
        assert!(matches!(
            policy.evaluate(
                &allowed,
                &o("http://dash.cloudflare.com"),
                Some(&o("http://dash.cloudflare.com"))
            ),
            Decision::Deny { .. }
        ));
    }

    #[test]
    fn evaluate_is_fail_closed_on_unknown_frame_origin() {
        let allowed = vec![o("https://login.example")];
        let policy = OriginPolicy::default();
        // A form inside a cross-origin iframe must not inherit the top-level
        // page's allowance.
        assert!(matches!(
            policy.evaluate(
                &allowed,
                &o("https://login.example"),
                Some(&o("https://attacker.example"))
            ),
            Decision::Deny { .. }
        ));
        assert!(matches!(
            policy.evaluate(&allowed, &o("https://login.example"), None),
            Decision::Deny { .. }
        ));
    }

    #[test]
    fn deny_rules_win_over_allowlist() {
        let allowed = vec![o("https://dash.example")];
        let policy = OriginPolicy::from_rules(vec![OriginRule {
            origin: o("https://dash.example"),
            mode: RuleMode::Deny,
        }]);
        assert!(matches!(
            policy.evaluate(
                &allowed,
                &o("https://dash.example"),
                Some(&o("https://dash.example"))
            ),
            Decision::Deny { .. }
        ));
    }

    #[test]
    fn classify_tiers() {
        let allowed = vec![o("https://dash.cloudflare.com")];
        let policy = OriginPolicy::default();
        assert_eq!(
            policy.classify(&allowed, &o("https://dash.cloudflare.com")),
            OriginMatch::Exact
        );
        assert_eq!(
            policy.classify(&allowed, &o("https://login.cloudflare.com")),
            OriginMatch::RegistrableDomain
        );
        assert_eq!(
            policy.classify(&allowed, &o("https://example.org")),
            OriginMatch::None
        );
        // Suffix lookalikes are not even a domain relationship.
        assert_eq!(
            policy.classify(&allowed, &o("https://notcloudflare.com")),
            OriginMatch::None
        );
    }

    #[test]
    fn redirect_verdict_escapes_on_allowlist_exit() {
        let allowed = vec![
            o("https://dash.cloudflare.com"),
            o("https://login.cloudflare.com"),
        ];
        let policy = OriginPolicy::default();
        assert_eq!(
            policy.redirect_verdict(&allowed, &o("https://login.cloudflare.com/oauth/ok")),
            RedirectVerdict::Continue
        );
        assert_eq!(
            policy.redirect_verdict(&allowed, &o("https://evil.example")),
            RedirectVerdict::InvalidateAndEscalate
        );
    }
}
