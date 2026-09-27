//! CDP Browser domain handler.
//!
//! Handles Browser.getVersion, Browser.close, Browser.getWindowForTarget.

use crate::domains::DomainResult;
use crate::protocol::CdpError;
use serde_json::{Value, json};

/// Dispatch Browser domain methods.
pub fn handle(method: &str, params: Option<Value>) -> DomainResult {
    match method {
        "getVersion" => get_version(),
        "close" => close(),
        "getWindowForTarget" => get_window_for_target(),
        "setWindowBounds" => set_window_bounds(params),
        "getWindowBounds" => Ok(Some(json!({
            "bounds": {
                "left": 0,
                "top": 0,
                "width": 1280,
                "height": 720,
                "windowState": "normal"
            }
        }))),
        _ => Err(CdpError {
            code: -32601,
            message: format!("Browser.{} not implemented", method),
        }),
    }
}

/// Browser.getVersion — returns browser and protocol version info.
fn get_version() -> DomainResult {
    let product = format!("OxiBrowser/{}", env!("CARGO_PKG_VERSION"));
    Ok(Some(json!({
        "protocolVersion": "1.3",
        "product": product,
        "revision": "@@revision",
        "userAgent": product,
        "jsVersion": env!("CARGO_PKG_VERSION")
    })))
}

/// Browser.close — closes the browser.
fn close() -> DomainResult {
    Ok(Some(json!({})))
}

/// Browser.setWindowBounds — install a viewport override from the bounds.
///
/// When both `width` and `height` are provided (and > 0), they call
/// `oxibrowser_core::session::set_viewport_override`, which takes effect on
/// the next navigation's layout — the already-loaded document is not
/// reflowed. Position (`left`/`top`) and `windowState` are accepted but
/// ignored (no OS window concept); `Browser.getWindowBounds` keeps reporting
/// the fixed default bounds.
fn set_window_bounds(params: Option<Value>) -> DomainResult {
    let params = params.unwrap_or_default();
    // CDP nests the size under `bounds`; a flat `width`/`height` is also
    // accepted for leniency.
    let bounds = params.get("bounds").unwrap_or(&params);
    let width = bounds.get("width").and_then(|v| v.as_f64());
    let height = bounds.get("height").and_then(|v| v.as_f64());
    if let (Some(w), Some(h)) = (width, height)
        && w > 0.0
        && h > 0.0
    {
        oxibrowser_core::session::set_viewport_override(w as u32, h as u32);
    }
    Ok(Some(json!({})))
}

/// Browser.getWindowForTarget — returns window ID for a target.
fn get_window_for_target() -> DomainResult {
    Ok(Some(json!({
        "windowId": 1,
        "bounds": {
            "left": 0,
            "top": 0,
            "width": 1280,
            "height": 720,
            "windowState": "normal"
        }
    })))
}
