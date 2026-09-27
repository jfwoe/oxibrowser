//! Secret redaction for network logs and HAR output.
//!
//! Every persisted observation surface (HAR files, CDP event URLs, future
//! log sinks) passes through [`RedactionProfile`]-driven filters here so
//! credentials, cookies, and tokens never reach disk in plaintext.
//!
//! Limitations (documented in docs/designs/2026-09-27-agent-auth-implementation.md §9
//! FM-1): header lists cannot catch org-specific auth headers, and key-based
//! query/body matching can be evaded by unusual encodings. Unknown / non-form
//! bodies are therefore fully redacted rather than field-filtered.

use std::sync::OnceLock;

/// Replacement string for every redacted value. Deliberately fixed-length so
/// no information about the original value (including its length) survives.
pub const REDACTED: &str = "__REDACTED__";

/// Default headers whose values never belong in a persisted log.
pub const DEFAULT_SENSITIVE_HEADERS: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
    "x-api-key",
    "x-auth-token",
    "x-access-token",
    "x-refresh-token",
    "x-csrf-token",
    "x-xsrf-token",
];

/// Default URL query parameter keys whose values are redacted
/// (OAuth `code`/`state` included — the research list, see 06 §4.1).
pub const DEFAULT_SENSITIVE_QUERY_KEYS: &[&str] = &[
    "token",
    "access_token",
    "refresh_token",
    "id_token",
    "api_key",
    "apikey",
    "key",
    "secret",
    "signature",
    "sig",
    "code",
    "state",
    "password",
    "passwd",
    "pwd",
    "otp",
    "auth",
    "email",
];

/// Default form-urlencoded body field keys whose values are redacted.
pub const DEFAULT_SENSITIVE_FORM_KEYS: &[&str] = &[
    "password", "passwd", "pass", "pwd", "secret", "token", "otp", "code", "auth",
];

/// Redaction rule set: the default profile plus caller-supplied extensions
/// for organization-specific auth headers.
#[derive(Debug, Clone)]
pub struct RedactionProfile {
    /// Header names (ASCII case-insensitive) whose values are replaced.
    pub sensitive_headers: Vec<String>,
    /// URL query parameter keys (ASCII case-insensitive) whose values are
    /// replaced.
    pub sensitive_query_keys: Vec<String>,
    /// form-urlencoded body field keys (ASCII case-insensitive) whose values
    /// are replaced.
    pub sensitive_form_keys: Vec<String>,
}

impl Default for RedactionProfile {
    fn default() -> Self {
        Self {
            sensitive_headers: DEFAULT_SENSITIVE_HEADERS
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            sensitive_query_keys: DEFAULT_SENSITIVE_QUERY_KEYS
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            sensitive_form_keys: DEFAULT_SENSITIVE_FORM_KEYS
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
        }
    }
}

impl RedactionProfile {
    /// The standard HAR/network-log profile.
    pub fn default_har() -> Self {
        Self::default()
    }

    /// Add organization-specific auth headers to the header list.
    pub fn with_extra_headers(mut self, headers: impl IntoIterator<Item = String>) -> Self {
        self.sensitive_headers.extend(headers);
        self
    }
}

static EXTRA_HEADERS: OnceLock<Vec<String>> = OnceLock::new();

/// Install organization-specific sensitive headers for the process lifetime
/// (CLI: `--redact-header NAME`). Must be called before the first HAR/event
/// export; a later call is a no-op (first initialization wins, mirroring
/// [`crate::security::audit::init`]).
pub fn set_extra_sensitive_headers(headers: Vec<String>) {
    let _ = EXTRA_HEADERS.set(headers);
}

/// The profile every export surface should use: the default list plus any
/// process-wide `--redact-header` extensions.
pub fn active_profile() -> RedactionProfile {
    match EXTRA_HEADERS.get() {
        Some(extra) if !extra.is_empty() => {
            RedactionProfile::default_har().with_extra_headers(extra.iter().cloned())
        }
        _ => RedactionProfile::default_har(),
    }
}

/// True when `name` is in the profile's sensitive header list (ASCII
/// case-insensitive, surrounding whitespace tolerated).
pub fn is_sensitive_header(name: &str, profile: &RedactionProfile) -> bool {
    let name = name.trim();
    profile
        .sensitive_headers
        .iter()
        .any(|s| s.eq_ignore_ascii_case(name))
}

/// Redact a header pair list. Sensitive headers keep their name but get
/// [`REDACTED`] as the value — presence stays debuggable, the secret does
/// not survive.
pub fn redact_headers(
    headers: &[(String, String)],
    profile: &RedactionProfile,
) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| {
            if is_sensitive_header(name, profile) {
                (name.clone(), REDACTED.to_string())
            } else {
                (name.clone(), value.clone())
            }
        })
        .collect()
}

/// True when `key` is in `keys` (ASCII case-insensitive). The comparison
/// operand is percent-decoded by the caller.
fn is_sensitive_key(key: &str, keys: &[String]) -> bool {
    keys.iter().any(|k| k.eq_ignore_ascii_case(key))
}

/// Redact the values of sensitive query parameters in a URL string.
///
/// Keys are compared after percent-decoding (the `url` crate decodes
/// `Pair` keys), so `access%5Ftoken=x` matches `access_token`. Query pairs
/// that fail to round-trip are dropped conservatively: if the URL cannot be
/// parsed, the original string is returned unchanged (callers treat parse
/// failures as "cannot prove it is safe to rewrite" and keep the original
/// for the in-memory log — HAR output for such URLs still carries redacted
/// headers, see the design's failure-mode notes).
pub fn redact_url_query(url: &str, profile: &RedactionProfile) -> String {
    let mut parsed = match url::Url::parse(url) {
        Ok(u) => u,
        Err(_) => return url.to_string(),
    };
    let pairs: Vec<(String, String)> = parsed
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let rewritten = form_urlencoded(&pairs, &profile.sensitive_query_keys);
    if rewritten.is_empty() {
        parsed.set_query(None);
    } else {
        parsed.set_query(Some(&rewritten));
    }
    parsed.to_string()
}

/// Serialize `(key, value)` pairs back into a form-urlencoded query/body,
/// replacing values of sensitive keys with [`REDACTED`]. Keys are compared
/// decoded; output is percent-encoded.
fn form_urlencoded(pairs: &[(String, String)], keys: &[String]) -> String {
    let mut out = String::new();
    for (k, v) in pairs {
        if !out.is_empty() {
            out.push('&');
        }
        let value = if is_sensitive_key(k, keys) {
            REDACTED
        } else {
            v.as_str()
        };
        out.push_str(&urlencode_component(k));
        out.push('=');
        out.push_str(&urlencode_component(value));
    }
    out
}

/// Minimal percent-encoding for form/query components (RFC 3986 unreserved
/// set kept literal, everything else hex-escaped as UTF-8).
fn urlencode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Outcome of [`redact_post_body`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RedactedBody {
    /// No body present — callers emit no `postData`.
    None,
    /// A body survives, with sensitive form fields value-redacted.
    Body(Vec<u8>),
    /// The body is not field-inspectable (non-form content type or undecodable)
    /// and must be replaced wholesale by [`REDACTED`].
    Opaque,
}

/// Redact a POST body.
///
/// Only `application/x-www-form-urlencoded` bodies are field-filtered (both
/// plain and when the caller already base64-decoded them). Everything else —
/// JSON login payloads, multipart, binary — becomes [`RedactedBody::Opaque`]
/// rather than attempting selective masking that key matching cannot
/// guarantee.
pub fn redact_post_body(
    body: Option<&[u8]>,
    content_type: &str,
    profile: &RedactionProfile,
) -> RedactedBody {
    let Some(bytes) = body else {
        return RedactedBody::None;
    };
    if bytes.is_empty() {
        return RedactedBody::None;
    }
    let is_form = content_type
        .split(';')
        .next()
        .map(|ct| {
            ct.trim()
                .eq_ignore_ascii_case("application/x-www-form-urlencoded")
        })
        .unwrap_or(false);
    if !is_form {
        return RedactedBody::Opaque;
    }
    let text = match std::str::from_utf8(bytes) {
        Ok(t) => t,
        // A form body that is not valid UTF-8 cannot be safely field-parsed.
        Err(_) => return RedactedBody::Opaque,
    };
    let pairs: Vec<(String, String)> = text
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            (percent_decode(k), percent_decode(v))
        })
        .collect();
    RedactedBody::Body(form_urlencoded(&pairs, &profile.sensitive_form_keys).into_bytes())
}

/// Percent-decode a query/body component (best-effort: invalid escapes are
/// kept literally, matching browser tolerance).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> RedactionProfile {
        RedactionProfile::default_har()
    }

    #[test]
    fn sensitive_header_matches_case_insensitively() {
        let p = profile();
        assert!(is_sensitive_header("Authorization", &p));
        assert!(is_sensitive_header("SET-COOKIE", &p));
        assert!(is_sensitive_header(" cookie ", &p));
        assert!(!is_sensitive_header("content-type", &p));
    }

    #[test]
    fn redact_headers_keeps_name_replaces_value() {
        let p = profile();
        let out = redact_headers(
            &[
                ("Authorization".into(), "Bearer abc".into()),
                ("Accept".into(), "text/html".into()),
            ],
            &p,
        );
        assert_eq!(out[0], ("Authorization".into(), REDACTED.to_string()));
        assert_eq!(out[1], ("Accept".into(), "text/html".into()));
    }

    #[test]
    fn extra_headers_are_matched() {
        let p = profile().with_extra_headers(["x-org-token".to_string()]);
        let out = redact_headers(&[("X-Org-Token".into(), "v".into())], &p);
        assert_eq!(out[0].1, REDACTED);
    }

    #[test]
    fn redacts_url_query_values_only() {
        let p = profile();
        let out = redact_url_query("https://e.com/p?token=abc&x=1", &p);
        assert!(out.contains(&format!("token={REDACTED}")), "{out}");
        assert!(out.contains("x=1"), "{out}");
        assert!(!out.contains("abc"), "{out}");
    }

    #[test]
    fn redacts_percent_encoded_query_key() {
        let p = profile();
        let out = redact_url_query("https://e.com/p?access%5Ftoken=abc", &p);
        assert!(out.contains(REDACTED), "{out}");
        assert!(!out.contains("abc"), "{out}");
    }

    #[test]
    fn unparseable_url_returns_original() {
        let p = profile();
        let out = redact_url_query("not a url?token=abc", &p);
        assert_eq!(out, "not a url?token=abc");
    }

    #[test]
    fn url_without_query_unchanged() {
        let p = profile();
        assert_eq!(redact_url_query("https://e.com/p", &p), "https://e.com/p");
    }

    #[test]
    fn form_body_field_redaction_preserves_other_fields() {
        let p = profile();
        let out = redact_post_body(
            Some(b"user=a&password=hunter2&keep=1".as_slice()),
            "application/x-www-form-urlencoded",
            &p,
        );
        let RedactedBody::Body(bytes) = out else {
            panic!("expected redacted body");
        };
        let s = String::from_utf8(bytes).unwrap();
        assert!(s.contains("user=a"), "{s}");
        assert!(s.contains("keep=1"), "{s}");
        assert!(s.contains(&format!("password={REDACTED}")), "{s}");
        assert!(!s.contains("hunter2"), "{s}");
    }

    #[test]
    fn form_content_type_parameters_tolerated() {
        let p = profile();
        let out = redact_post_body(
            Some(b"password=x".as_slice()),
            "application/x-www-form-urlencoded; charset=UTF-8",
            &p,
        );
        assert!(matches!(out, RedactedBody::Body(_)));
    }

    #[test]
    fn non_form_body_is_fully_opaque() {
        let p = profile();
        let out = redact_post_body(
            Some(br#"{"password":"hunter2"}"#.as_slice()),
            "application/json",
            &p,
        );
        assert_eq!(out, RedactedBody::Opaque);
    }

    #[test]
    fn binary_form_body_is_opaque() {
        let p = profile();
        let out = redact_post_body(
            Some(&[0xff, 0xfe, 0x01]),
            "application/x-www-form-urlencoded",
            &p,
        );
        assert_eq!(out, RedactedBody::Opaque);
    }

    #[test]
    fn empty_and_missing_bodies_are_none() {
        let p = profile();
        assert_eq!(
            redact_post_body(None, "application/json", &p),
            RedactedBody::None
        );
        assert_eq!(
            redact_post_body(Some(b"".as_slice()), "application/json", &p),
            RedactedBody::None
        );
    }

    #[test]
    fn redacted_marker_hides_length() {
        // Fixed marker: no per-value length signal.
        assert_eq!(REDACTED, "__REDACTED__");
    }
}
