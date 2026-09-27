//! Stable element references (`e{N}`) for the OXI domain.
//!
//! `OXI.getInteractiveElements` / `OXI.ariaSnapshot` hand out short ref ids
//! (`e1`, `e2`, …) per observed element. Each ref records the node id, the
//! document generation, a content fingerprint, and a CSS selector so
//! `OXI.clickRef` / `OXI.fillRef` / `OXI.waitRef` can detect drift between
//! observation and use (stale refs → `OXI.` handlers answer
//! "stale ref — re-observe").
//!
//! Refs are keyed per browsing session (`Session::id` rendered as a string)
//! and live in a process-global registry; `clear_session` drops one
//! session's refs and resets its counter.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

/// One observed element behind a stable ref.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefEntry {
    /// DomSnapshot node id at observation time.
    pub node_id: u32,
    /// Document generation (`Page::generation`) at observation time.
    pub generation: u64,
    /// Content fingerprint (`DomSnapshot::fingerprint`) at observation time.
    pub fingerprint: String,
    /// CSS selector path usable with `document.querySelector`.
    pub selector: String,
}

/// Per-session ref bookkeeping: ref → entry, plus the monotonic counter.
#[derive(Debug, Default)]
struct RefSessionState {
    map: HashMap<String, RefEntry>,
    counter: u64,
}

/// Process-global registry, keyed by session key (`Session::id` string).
static REFS: LazyLock<Mutex<HashMap<String, RefSessionState>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn lock() -> std::sync::MutexGuard<'static, HashMap<String, RefSessionState>> {
    REFS.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Allocation / resolution of stable element refs.
pub struct RefRegistry;

impl RefRegistry {
    /// Allocate the next ref (`e{N}`) for `session_key`, recording `node_id`,
    /// `generation`, `fingerprint`, and `selector`.
    pub fn allocate(
        session_key: &str,
        node_id: u32,
        generation: u64,
        fingerprint: String,
        selector: String,
    ) -> String {
        let mut registry = lock();
        let state = registry.entry(session_key.to_string()).or_default();
        state.counter += 1;
        let r#ref = format!("e{}", state.counter);
        state.map.insert(
            r#ref.clone(),
            RefEntry {
                node_id,
                generation,
                fingerprint,
                selector,
            },
        );
        r#ref
    }

    /// Resolve `r#ref` for `session_key`, or `Err` when the session has no
    /// such ref (never observed, or the session was cleared).
    pub fn resolve(session_key: &str, r#ref: &str) -> Result<RefEntry, String> {
        lock()
            .get(session_key)
            .and_then(|state| state.map.get(r#ref))
            .cloned()
            .ok_or_else(|| format!("unknown ref: {}", r#ref))
    }

    /// Drop every ref allocated for `session_key` (counter restarts at 0).
    pub fn clear_session(session_key: &str) {
        lock().remove(session_key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocate_resolve_roundtrip_and_isolation() {
        let key = "session-1";
        RefRegistry::clear_session(key);

        let first = RefRegistry::allocate(key, 7, 3, "button||Hit".into(), "button".into());
        assert_eq!(first, "e1");
        let entry = RefRegistry::resolve(key, "e1").expect("resolve first");
        assert_eq!(entry.node_id, 7);
        assert_eq!(entry.generation, 3);
        assert_eq!(entry.fingerprint, "button||Hit");
        assert_eq!(entry.selector, "button");

        let second = RefRegistry::allocate(key, 8, 4, "a||Home".into(), "a".into());
        assert_eq!(second, "e2");
        assert_eq!(RefRegistry::resolve(key, "e2").unwrap().node_id, 8);

        // Unknown refs and other sessions reject.
        assert!(RefRegistry::resolve(key, "e99").is_err());
        assert!(RefRegistry::resolve("session-other", "e1").is_err());

        // Clearing drops the session's refs and resets the counter.
        RefRegistry::clear_session(key);
        assert!(RefRegistry::resolve(key, "e1").is_err());
        assert_eq!(
            RefRegistry::allocate(key, 1, 1, String::new(), String::new()),
            "e1"
        );
        RefRegistry::clear_session(key);
    }
}
