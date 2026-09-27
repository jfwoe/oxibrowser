//! OXI domain — OxiBrowser AI agent extensions.
//!
//! Provides AI-agent-friendly methods beyond standard CDP:
//! - `OXI.getMarkdown` — page content as Markdown
//! - `OXI.getPageInfo` — URL, title, status
//! - `OXI.getStructuredPage` — headings, links, meta as structured JSON
//! - `OXI.getAccessibilityTree` — semantic tree of what's on the page
//! - `OXI.getInteractiveElements` — interactive elements in document order,
//!   each with a stable `ref` for the `OXI.*Ref` actions
//! - `OXI.getBoxModelScreenshot` — PNG with colored boxes for each element
//! - `OXI.clickRef` / `OXI.fillRef` / `OXI.waitRef` — act on stable refs
//!   (generation + fingerprint validated; drift answers "stale ref —
//!   re-observe")
//! - `OXI.ariaSnapshot` — Playwright-style YAML of the visible tree, with
//!   `[ref=eN]` markers on interactive elements
//! - `OXI.exportStorageState` / `OXI.importStorageState` — cookies +
//!   localStorage snapshot round-trip (Playwright-compatible `StorageState`)

use crate::domains::{DispatchContext, DomainResult};
use crate::protocol::CdpError;
use crate::refs::RefRegistry;
use serde_json::{Value, json};

/// Handle OXI domain methods.
pub async fn handle(method: &str, params: Option<Value>, ctx: &DispatchContext) -> DomainResult {
    match method {
        "getMarkdown" => get_markdown(ctx).await,
        "getPageInfo" => get_page_info(ctx).await,
        "getStructuredPage" => get_structured_page(params, ctx).await,
        "getAccessibilityTree" => get_accessibility_tree(ctx).await,
        "getInteractiveElements" => get_interactive_elements(ctx).await,
        "getBoxModelScreenshot" => get_box_model_screenshot(params, ctx).await,
        "clickRef" => click_ref(params, ctx).await,
        "fillRef" => fill_ref(params, ctx).await,
        "waitRef" => wait_ref(params, ctx).await,
        "ariaSnapshot" => aria_snapshot(params, ctx).await,
        "exportStorageState" => export_storage_state(ctx).await,
        "importStorageState" => import_storage_state(params, ctx).await,
        "getApiGaps" => get_api_gaps(params, ctx).await,
        _ => Err(CdpError {
            code: -32601,
            message: format!("unknown method: OXI.{}", method),
        }),
    }
}

async fn get_markdown(ctx: &DispatchContext) -> DomainResult {
    let guard = ctx.session.read().await;
    let markdown = guard.page().map(|p| p.to_markdown()).unwrap_or_default();
    Ok(Some(json!({ "markdown": markdown })))
}

async fn get_page_info(ctx: &DispatchContext) -> DomainResult {
    let guard = ctx.session.read().await;
    let url = guard
        .current_url()
        .map(|u| u.to_string())
        .unwrap_or_default();
    let title = guard
        .page()
        .and_then(|p| p.title().map(|t| t.to_string()))
        .unwrap_or_default();
    let status = guard.page().map(|p| p.status()).unwrap_or(0);
    Ok(Some(json!({
        "url": url,
        "title": title,
        "status": status,
        "readyState": "complete"
    })))
}

/// OXI.getStructuredPage — return structured page data.
///
/// Returns headings, links, meta tags, and basic page info as JSON.
/// This is optimized for AI agent consumption.
///
/// Optional params:
/// - `maxLinks` (number): limit number of links returned (default: 200)
async fn get_structured_page(_params: Option<Value>, ctx: &DispatchContext) -> DomainResult {
    let max_links = _params
        .as_ref()
        .and_then(|p| p.get("maxLinks"))
        .and_then(|v| v.as_u64())
        .unwrap_or(200) as usize;

    let mut guard = ctx.session.write().await;
    let url = guard
        .current_url()
        .map(|u| u.to_string())
        .unwrap_or_default();
    let snapshot = guard.dom_snapshot().await?;
    let title = snapshot
        .as_ref()
        .map(|s| s.title.clone())
        .unwrap_or_default();

    let (headings, links, meta) = match snapshot {
        Some(s) => {
            let headings: Vec<Value> = s
                .headings()
                .into_iter()
                .map(|(level, text)| json!({ "level": level, "text": text }))
                .collect();
            let links: Vec<Value> = s
                .links()
                .into_iter()
                .take(max_links)
                .map(|(text, href)| json!({ "text": text, "href": href }))
                .collect();
            let meta: Value = s
                .meta_tags()
                .into_iter()
                .map(|(k, v)| (k, json!(v)))
                .collect();
            (headings, links, meta)
        }
        None => (vec![], vec![], json!({})),
    };

    Ok(Some(json!({
        "url": url,
        "title": title,
        "headings": headings,
        "links": links,
        "meta": meta,
        "linkCount": links.len(),
        "headingCount": headings.len(),
    })))
}

/// OXI.getAccessibilityTree — return semantic tree of page content.
///
/// Shows what a user (or screen reader) would perceive:
/// roles, labels, visibility, interactivity, approximate positions.
async fn get_accessibility_tree(ctx: &DispatchContext) -> DomainResult {
    let mut guard = ctx.session.write().await;
    let snapshot = guard.dom_snapshot().await?;

    let tree = match snapshot {
        Some(s) => oxibrowser_core::css::render_accessibility_tree(&s),
        None => "(no page loaded)".into(),
    };

    Ok(Some(json!({ "tree": tree })))
}

/// OXI.getInteractiveElements — list interactive elements in document order.
///
/// One entry per element that is interactive: tag in
/// a/button/input/select/textarea, an `onclick` attribute, an interactive
/// `role` (button/link/tab/checkbox/radio), or `tabindex >= 0`. Detection and
/// role/selector computation live in `oxibrowser_core::js::dom_snapshot`.
///
/// Each entry additionally carries a stable `ref` (`e{N}`) usable with
/// `OXI.clickRef` / `OXI.fillRef` / `OXI.waitRef`; the top-level response
/// includes the page `generation` the refs were issued against.
async fn get_interactive_elements(ctx: &DispatchContext) -> DomainResult {
    let mut guard = ctx.session.write().await;
    let session_key = guard.id().to_string();
    let snapshot = guard.dom_snapshot().await?;
    let generation = guard.page().map(|p| p.generation()).unwrap_or(0);

    let elements = match &snapshot {
        Some(s) => {
            let items = s.interactive_elements();
            let mut out = Vec::with_capacity(items.len());
            for el in items {
                let r#ref = RefRegistry::allocate(
                    &session_key,
                    el.node_id,
                    generation,
                    s.fingerprint(el.node_id),
                    el.selector.clone(),
                );
                let mut value = serde_json::to_value(&el).unwrap_or(Value::Null);
                if let Value::Object(map) = &mut value {
                    map.insert("ref".into(), json!(r#ref));
                }
                out.push(value);
            }
            out
        }
        None => vec![],
    };

    Ok(Some(
        json!({ "elements": elements, "generation": generation }),
    ))
}

/// OXI.getBoxModelScreenshot — PNG with colored boxes for each element.
///
/// Uses LayoutEngine to estimate positions and draws:
/// - Background-colored rectangles for each visible element
/// - Text content inside boxes
/// - Element borders
async fn get_box_model_screenshot(params: Option<Value>, ctx: &DispatchContext) -> DomainResult {
    let params = params.unwrap_or_default();
    let viewport_width = params
        .get("viewportWidth")
        .and_then(|v| v.as_u64())
        .unwrap_or(1280) as u32;

    let mut guard = ctx.session.write().await;
    let snapshot = guard.dom_snapshot().await?;

    let png_bytes = match snapshot {
        Some(s) => {
            oxibrowser_core::css::render_box_model_png(&s, viewport_width).unwrap_or_default()
        }
        None => Vec::new(),
    };

    use base64::Engine;
    let data = base64::engine::general_purpose::STANDARD.encode(&png_bytes);

    Ok(Some(json!({
        "data": data,
        "metadata": {
            "pageScaleFactor": 1,
            "deviceWidth": viewport_width,
        }
    })))
}

// ── Stable refs (OXI.*Ref) ──────────────────────────────────────────────────

/// The `stale ref` rejection: the page (generation or element content) drifted
/// since the ref was issued; the agent must re-observe.
fn stale_ref_error() -> CdpError {
    CdpError {
        code: -32000,
        message: "stale ref — re-observe".to_string(),
    }
}

fn string_param<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params.get(key).and_then(|v| v.as_str())
}

fn missing_param(param: &str) -> CdpError {
    CdpError {
        code: -32602,
        message: format!("requires a string '{param}' parameter"),
    }
}

/// Shared ref-validation flow: resolve → rebuild the current snapshot →
/// compare the recorded generation + fingerprint. Any drift (navigation,
/// mutation) rejects with "stale ref — re-observe"; on match the entry and the
/// fresh snapshot are returned for the action.
async fn resolve_and_validate(
    session: &mut oxibrowser_core::session::Session,
    session_key: &str,
    r#ref: &str,
) -> Result<(crate::refs::RefEntry, oxibrowser_core::js::DomSnapshot), CdpError> {
    let entry = RefRegistry::resolve(session_key, r#ref).map_err(|e| CdpError {
        code: -32602,
        message: e,
    })?;
    let generation = session.page().map(|p| p.generation()).unwrap_or(0);
    let snapshot = session.dom_snapshot().await?.ok_or_else(stale_ref_error)?;
    if generation != entry.generation || snapshot.fingerprint(entry.node_id) != entry.fingerprint {
        return Err(stale_ref_error());
    }
    Ok((entry, snapshot))
}

/// Click JS mirroring `oxibrowser_core::tab::Tab::click` — the core Tab path
/// isn't reachable from the CDP layer, so the click snippet runs directly via
/// session evaluate (same pattern as the `Input.*` CdpStubs handlers).
fn click_js(selector: &str) -> String {
    let sel_json = serde_json::to_string(selector).unwrap_or_default();
    format!(
        r#"(function() {{
            var el = document.querySelector({sel_json});
            if (!el) return null;
            var rect = el.getBoundingClientRect
                ? el.getBoundingClientRect()
                : {{ left: 0, top: 0, width: 0, height: 0 }};
            var x = rect.left + rect.width / 2;
            var y = rect.top + rect.height / 2;
            el.dispatchEvent(new MouseEvent('click', {{
                bubbles: true,
                cancelable: true,
                clientX: x,
                clientY: y,
                button: 0
            }}));
            return el.tagName;
        }})()"#,
    )
}

/// OXI.clickRef — click an element through a stable ref.
///
/// Resolves `ref` (session key: the `sessionKey` param when given, else this
/// session's id), validates generation + fingerprint against the current
/// snapshot, then dispatches a click MouseEvent at the element's center.
async fn click_ref(params: Option<Value>, ctx: &DispatchContext) -> DomainResult {
    let params = params.ok_or_else(|| CdpError {
        code: -32602,
        message: "clickRef requires parameters".to_string(),
    })?;
    let r#ref = string_param(&params, "ref")
        .ok_or_else(|| missing_param("ref"))?
        .to_string();

    let mut guard = ctx.session.write().await;
    let session_key = string_param(&params, "sessionKey")
        .map(str::to_string)
        .unwrap_or_else(|| guard.id().to_string());

    let (entry, _snapshot) = resolve_and_validate(&mut guard, &session_key, &r#ref).await?;

    let js = click_js(&entry.selector);
    let result = guard.evaluate_js(&js).await?;
    if result.value.as_ref().is_none_or(|v| v.is_null()) {
        return Err(CdpError {
            code: -32000,
            message: format!("clickRef: no element matching '{}'", entry.selector),
        });
    }
    Ok(Some(json!({ "clicked": true, "ref": r#ref })))
}

/// OXI.fillRef — fill a form control through a stable ref.
///
/// Same resolve + generation/fingerprint validation as `clickRef`, then runs
/// the core `js_fill` snippet against the recorded selector.
async fn fill_ref(params: Option<Value>, ctx: &DispatchContext) -> DomainResult {
    let params = params.ok_or_else(|| CdpError {
        code: -32602,
        message: "fillRef requires parameters".to_string(),
    })?;
    let r#ref = string_param(&params, "ref")
        .ok_or_else(|| missing_param("ref"))?
        .to_string();
    let value = string_param(&params, "value")
        .ok_or_else(|| missing_param("value"))?
        .to_string();

    let mut guard = ctx.session.write().await;
    let session_key = string_param(&params, "sessionKey")
        .map(str::to_string)
        .unwrap_or_else(|| guard.id().to_string());

    let (entry, _snapshot) = resolve_and_validate(&mut guard, &session_key, &r#ref).await?;

    let js = oxibrowser_core::js::form::js_fill(&entry.selector, &value);
    guard.evaluate_js(&js).await?;
    Ok(Some(json!({ "filled": true, "ref": r#ref })))
}

/// OXI.waitRef — wait until the ref's selector appears in the snapshot.
///
/// Polls `dom_snapshot` every 50 ms (the `Tab::wait_for` equivalent) until the
/// selector matches or `timeoutMs` (default 5000) elapses. Unlike
/// click/fill, absence is not stale — it is what waiting is for.
async fn wait_ref(params: Option<Value>, ctx: &DispatchContext) -> DomainResult {
    let params = params.ok_or_else(|| CdpError {
        code: -32602,
        message: "waitRef requires parameters".to_string(),
    })?;
    let r#ref = string_param(&params, "ref")
        .ok_or_else(|| missing_param("ref"))?
        .to_string();
    let timeout_ms = params
        .get("timeoutMs")
        .and_then(|v| v.as_u64())
        .unwrap_or(5000);

    let mut guard = ctx.session.write().await;
    let session_key = string_param(&params, "sessionKey")
        .map(str::to_string)
        .unwrap_or_else(|| guard.id().to_string());

    let entry = RefRegistry::resolve(&session_key, &r#ref).map_err(|e| CdpError {
        code: -32602,
        message: e,
    })?;

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    loop {
        let found = guard
            .dom_snapshot()
            .await?
            .is_some_and(|s| s.query_selector(&entry.selector).is_some());
        if found {
            return Ok(Some(json!({ "found": true, "ref": r#ref })));
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(CdpError {
                code: -32000,
                message: format!(
                    "waitRef: timeout after {timeout_ms}ms waiting for '{}'",
                    entry.selector
                ),
            });
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// OXI.ariaSnapshot — Playwright-style YAML of the visible tree.
///
/// Renders visible elements as `- role "name" [flags]` / `- role:` container
/// lines (`oxibrowser_core::js::DomSnapshot::render_aria_yaml`). Interactive
/// elements additionally get stable refs from the same registry as
/// `getInteractiveElements`, so the snapshot is directly actionable via the
/// `OXI.*Ref` methods.
async fn aria_snapshot(_params: Option<Value>, ctx: &DispatchContext) -> DomainResult {
    let mut guard = ctx.session.write().await;
    let session_key = guard.id().to_string();
    let generation = guard.page().map(|p| p.generation()).unwrap_or(0);
    let snapshot = guard.dom_snapshot().await?;

    let yaml = match snapshot {
        Some(s) => s.render_aria_yaml(&mut |node_id| {
            let node = s.nodes.get(&node_id)?;
            if !oxibrowser_core::js::dom_snapshot::is_interactive_element(node) {
                return None;
            }
            Some(RefRegistry::allocate(
                &session_key,
                node_id,
                generation,
                s.fingerprint(node_id),
                s.css_selector_path(node_id),
            ))
        }),
        None => String::new(),
    };

    Ok(Some(json!({ "snapshot": yaml })))
}

// ── Storage state (OXI.exportStorageState / OXI.importStorageState) ─────────

/// OXI.exportStorageState — cookies + per-origin localStorage snapshot
/// (Playwright-compatible `StorageState`).
async fn export_storage_state(ctx: &DispatchContext) -> DomainResult {
    let guard = ctx.session.read().await;
    let state = guard.export_state();
    let value = serde_json::to_value(&state).map_err(|e| CdpError {
        code: -32603,
        message: format!("exportStorageState: {e}"),
    })?;
    Ok(Some(json!({ "state": value })))
}

/// OXI.importStorageState — seed cookies + localStorage from a prior export.
async fn import_storage_state(params: Option<Value>, ctx: &DispatchContext) -> DomainResult {
    let params = params.ok_or_else(|| CdpError {
        code: -32602,
        message: "importStorageState requires parameters".to_string(),
    })?;
    let state_value = params.get("state").cloned().ok_or_else(|| CdpError {
        code: -32602,
        message: "importStorageState requires a 'state' object".to_string(),
    })?;
    let state: oxibrowser_core::StorageState =
        serde_json::from_value(state_value).map_err(|e| CdpError {
            code: -32602,
            message: format!("importStorageState: invalid state — {e}"),
        })?;

    let mut guard = ctx.session.write().await;
    guard.import_state(&state)?;
    Ok(Some(json!({})))
}

/// OXI.getApiGaps — JS/DOM API coverage telemetry (#15).
///
/// Returns the process-global snapshot of unsupported/polyfilled Web API
/// accesses recorded when the browser runs with
/// [`oxibrowser_core::BrowserConfig::telemetry`]: reads of `window`
/// properties the engine does not implement, keyed by property name.
/// Response: `{"gaps": [{"name", "count"}, …]}` sorted by count
/// (descending). The snapshot is read-only — counters are **not** reset,
/// so repeated calls observe the cumulative process totals.
async fn get_api_gaps(_params: Option<Value>, _ctx: &DispatchContext) -> DomainResult {
    let gaps = api_gaps_payload();
    Ok(Some(gaps))
}

/// Assemble the `getApiGaps` response body from the core telemetry
/// snapshot (count-descending).
fn api_gaps_payload() -> Value {
    let gaps: Vec<Value> = oxibrowser_core::js::runtime::telemetry_snapshot()
        .into_iter()
        .map(|(name, count)| json!({ "name": name, "count": count }))
        .collect();
    json!({ "gaps": gaps })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// OXI.getApiGaps reflects the core telemetry snapshot: entries ordered
    /// by count descending, shape `{"gaps": [{"name", "count"}]}`, and
    /// read-only (snapshotting does not reset the counters).
    #[test]
    fn test_get_api_gaps_payload_sorted_and_read_only() {
        oxibrowser_core::js::runtime::telemetry_reset();
        let rec = oxibrowser_core::js::runtime::telemetry_record;
        rec("IntersectionObserver");
        rec("webkitRequestAnimationFrame");
        rec("webkitRequestAnimationFrame");

        let payload = api_gaps_payload();
        let gaps = payload["gaps"].as_array().expect("gaps array");
        assert!(gaps.len() >= 2, "both recorded names present");
        assert_eq!(gaps[0]["name"], "webkitRequestAnimationFrame");
        assert_eq!(gaps[0]["count"], 2, "highest count first");
        assert_eq!(gaps[1]["name"], "IntersectionObserver");
        assert_eq!(gaps[1]["count"], 1);

        // Read-only: a second snapshot still sees the same totals.
        let again = api_gaps_payload();
        assert_eq!(again["gaps"][0]["count"], 2, "snapshot does not reset");

        oxibrowser_core::js::runtime::telemetry_reset();
    }
}
