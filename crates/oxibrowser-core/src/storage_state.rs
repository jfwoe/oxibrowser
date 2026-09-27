//! Playwright-compatible storage state: cookies + per-origin localStorage.
//!
//! [`StorageState`] is the serializable snapshot exchanged via
//! [`crate::session::Session::export_state`] / [`crate::session::Session::import_state`].
//! The JSON shape follows Playwright's `storageState` (`cookies` +
//! `origins[].localStorage`), so snapshots are hand-writable and diffable.
//!
//! # Example
//!
//! ```
//! use oxibrowser_core::storage_state::StorageState;
//! let json = r#"{"cookies":[],"origins":[{"origin":"https://example.com","localStorage":[{"name":"session","value":"abc"}]}]}"#;
//! let st: StorageState = serde_json::from_str(json).unwrap();
//! assert_eq!(st.origins[0].local_storage[0].name, "session");
//! ```

use crate::network::cookie::CookieEntry;
use serde::{Deserialize, Serialize};

/// Persisted browser storage state: cookies plus per-origin localStorage.
///
/// Both fields default to empty so a partial snapshot (cookies only, or
/// storage only) deserializes without ceremony.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct StorageState {
    #[serde(default)]
    pub cookies: Vec<CookieEntry>,
    #[serde(default)]
    pub origins: Vec<OriginState>,
}

/// localStorage entries for one origin.
///
/// Playwright spells each entry `{"name": ..., "value": ...}` — mirror that
/// exactly so snapshots are interchangeable.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct LocalStorageEntry {
    pub name: String,
    pub value: String,
}

/// localStorage entries for one origin.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct OriginState {
    pub origin: String,
    #[serde(rename = "localStorage")]
    pub local_storage: Vec<LocalStorageEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `CookieEntry::same_site` must round-trip through the Playwright
    /// spelling (`"Lax"` / `"Strict"` / `"None"`): serde's derived unit-variant
    /// representation is the variant name, which already matches.
    #[test]
    fn same_site_serializes_playwright_compatible() {
        let json = serde_json::to_string(&Some(crate::network::cookie::SameSite::Lax)).unwrap();
        assert_eq!(json, r#""Lax""#);
        let json = serde_json::to_string(&Some(crate::network::cookie::SameSite::Strict)).unwrap();
        assert_eq!(json, r#""Strict""#);
        let json = serde_json::to_string(&Some(crate::network::cookie::SameSite::None)).unwrap();
        assert_eq!(json, r#""None""#);
        let back: Option<crate::network::cookie::SameSite> =
            serde_json::from_str(r#""Lax""#).unwrap();
        assert_eq!(back, Some(crate::network::cookie::SameSite::Lax));
    }

    #[test]
    fn storage_state_round_trips_through_json() {
        let st = StorageState {
            cookies: vec![CookieEntry {
                name: "sid".into(),
                value: "abc".into(),
                path: Some("/".into()),
                domain: Some("example.com".into()),
                secure: true,
                http_only: true,
                same_site: Some(crate::network::cookie::SameSite::Lax),
                ..Default::default()
            }],
            origins: vec![OriginState {
                origin: "https://example.com".into(),
                local_storage: vec![LocalStorageEntry {
                    name: "k".into(),
                    value: "v".into(),
                }],
            }],
        };
        let json = serde_json::to_string(&st).unwrap();
        assert!(
            json.contains(r#""sameSite":"Lax""#),
            "sameSite must keep the Playwright spelling: {json}"
        );
        assert!(
            json.contains(r#""localStorage""#),
            "localStorage key must be camelCase: {json}"
        );
        assert!(
            json.contains(r#"{"name":"k","value":"v"}"#),
            "entries must use the Playwright name/value object form: {json}"
        );
        let back: StorageState = serde_json::from_str(&json).unwrap();
        assert_eq!(back.cookies[0].name, "sid");
        assert_eq!(
            back.cookies[0].same_site,
            Some(crate::network::cookie::SameSite::Lax)
        );
        assert_eq!(
            back.origins[0].local_storage[0],
            LocalStorageEntry {
                name: "k".into(),
                value: "v".into()
            }
        );
    }

    #[test]
    fn partial_snapshots_deserialize_with_defaults() {
        let st: StorageState = serde_json::from_str(r#"{"cookies":[]}"#).unwrap();
        assert!(st.origins.is_empty());
        let st: StorageState = serde_json::from_str("{}").unwrap();
        assert!(st.cookies.is_empty() && st.origins.is_empty());
    }
}
