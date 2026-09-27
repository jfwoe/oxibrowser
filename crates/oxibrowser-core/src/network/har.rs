//! HAR 1.2 (HTTP Archive) serialization for the session request log.
//!
//! [`to_har_json`] converts a [`RequestRecord`] slice into the HAR 1.2
//! `log` JSON structure understood by Chrome DevTools, HAR viewers, and
//! `Network.getHar`-style consumers. Bodies are base64-encoded; unknown
//! sizes use the spec's `-1` sentinel.

use crate::security::redact::{
    RedactedBody, RedactionProfile, redact_headers, redact_post_body, redact_url_query,
};
use crate::session::RequestRecord;
use base64::Engine as _;
use serde_json::{Value, json};

/// Creator name stamped into every archive.
const CREATOR_NAME: &str = "oxibrowser";

/// HAR `content.size` / `bodySize` sentinel for "unknown".
const SIZE_UNKNOWN: i64 = -1;

/// Redaction profile applied to every default HAR export: the default header
/// list plus any process-wide `--redact-header` extensions. Raw export
/// (`--har-raw`) bypasses this deliberately and is audit-logged by the CLI.
fn har_redaction_profile() -> RedactionProfile {
    crate::security::redact::active_profile()
}

/// Format an epoch-milliseconds timestamp as ISO 8601 / RFC 3339 with
/// millisecond precision and a `Z` offset (`2026-01-02T03:04:05.678Z`) —
/// the `startedDateTime` format HAR consumers expect.
fn iso8601(epoch_ms: f64) -> String {
    let total_ms = epoch_ms as i64;
    let secs = total_ms.div_euclid(1000);
    let millis = total_ms.rem_euclid(1000) as u32;
    let nanos = millis * 1_000_000;
    chrono::DateTime::from_timestamp(secs, nanos)
        .unwrap_or(chrono::DateTime::UNIX_EPOCH)
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Convert request-log records into a complete HAR 1.2 `log` document.
///
/// Records are emitted in log order (oldest first). Entries are synthesized
/// from the data captured by the session request log — fields the session
/// does not observe (query strings, header byte sizes, redirect URLs) use
/// the spec's empty/-1 placeholders.
///
/// **Redaction is on by default**: `Authorization`/`Cookie`/`Set-Cookie` and
/// other sensitive headers, sensitive URL query values, and non-form POST
/// bodies are replaced before serialization. Use [`to_har_json_raw`] only
/// for explicit debug opt-in (`--har-raw`).
pub fn to_har_json(records: &[RequestRecord]) -> Value {
    to_har_json_impl(records, Some(&har_redaction_profile()))
}

/// Convert records into HAR JSON with **no redaction**. Values include
/// cookies, bearer tokens, and POST bodies verbatim. Callers must treat the
/// result as a credential-bearing artifact (CLI warns and audit-logs).
pub fn to_har_json_raw(records: &[RequestRecord]) -> Value {
    to_har_json_impl(records, None)
}

fn to_har_json_impl(records: &[RequestRecord], redaction: Option<&RedactionProfile>) -> Value {
    let entries: Vec<Value> = records
        .iter()
        .map(|rec| request_record_to_entry(rec, redaction))
        .collect();
    json!({
        "log": {
            "version": "1.2",
            "creator": {
                "name": CREATOR_NAME,
                "version": env!("CARGO_PKG_VERSION"),
            },
            "entries": entries,
        }
    })
}

/// Convert one [`RequestRecord`] into a HAR `entries[]` element.
///
/// `redaction` drives the secret filters; `None` produces the raw (unsafe)
/// form used only by `--har-raw`.
fn request_record_to_entry(rec: &RequestRecord, redaction: Option<&RedactionProfile>) -> Value {
    let started_date_time = iso8601(rec.started_at_ms);
    // Total time from request start to response finish; unfinished (in-flight
    // or failed without a finish stamp) entries report 0.
    let time_ms = rec
        .finished_at_ms
        .map_or(0.0, |finished| (finished - rec.started_at_ms).max(0.0));

    let post_body_size = rec
        .post_body
        .as_ref()
        .map_or(SIZE_UNKNOWN, |b| b.len() as i64);
    let content_type = request_content_type(rec);
    // Post-data under redaction: form bodies are field-redacted then
    // base64-encoded as usual; non-form bodies become the fixed REDACTED
    // marker (no base64, no length signal). Raw export keeps the verbatim
    // base64 body.
    let post_data = match (rec.post_body.as_deref(), redaction) {
        (None, _) => None,
        (Some(body), Some(profile)) => match redact_post_body(Some(body), &content_type, profile) {
            RedactedBody::None => None,
            RedactedBody::Body(redacted) => Some(post_data_json(
                &content_type,
                &base64::engine::general_purpose::STANDARD.encode(redacted),
            )),
            RedactedBody::Opaque => Some(post_data_json(
                &content_type,
                crate::security::redact::REDACTED,
            )),
        },
        (Some(body), None) => Some(post_data_json(
            &content_type,
            &base64::engine::general_purpose::STANDARD.encode(body),
        )),
    };

    let map_headers = |headers: &[(String, String)]| -> Vec<Value> {
        let mapped = match redaction {
            Some(profile) => redact_headers(headers, profile),
            None => headers.to_vec(),
        };
        mapped
            .iter()
            .map(|(name, value)| json!({ "name": name, "value": value }))
            .collect()
    };
    let request_headers = map_headers(&rec.request_headers);
    let response_headers = map_headers(&rec.response_headers);

    let url = match redaction {
        Some(profile) => redact_url_query(&rec.url, profile),
        None => rec.url.clone(),
    };

    let mut request = json!({
        "method": rec.method,
        "url": url,
        "httpVersion": "HTTP/1.1",
        "headers": request_headers,
        "queryString": [],
        "headersSize": SIZE_UNKNOWN,
        "bodySize": post_body_size,
    });
    if let Some(post_data) = post_data {
        request["postData"] = post_data;
    }

    json!({
        "startedDateTime": started_date_time,
        "time": time_ms,
        "request": request,
        "response": {
            "status": rec.status.unwrap_or(0),
            "statusText": "",
            "httpVersion": "HTTP/1.1",
            "headers": response_headers,
            "content": {
                // Body byte length when the body was read; -1 while unknown.
                "size": rec.response_body_length.map_or(SIZE_UNKNOWN, |n| n as i64),
                "mimeType": rec.mime_type,
            },
            "redirectURL": "",
            "headersSize": SIZE_UNKNOWN,
            "bodySize": SIZE_UNKNOWN,
        },
        "cache": {},
        "timings": {
            "send": 0,
            // The session records start/finish wall-clock stamps only; all
            // observed time is reported as wait.
            "wait": time_ms,
            "receive": 0,
        },
    })
}

/// Build the HAR `postData` object: base64-encoded body text (or the plain
/// REDACTED marker for opaque redaction).
fn post_data_json(content_type: &str, text: &str) -> Value {
    if text == crate::security::redact::REDACTED {
        json!({ "mimeType": content_type, "text": text })
    } else {
        json!({ "mimeType": content_type, "text": text, "encoding": "base64" })
    }
}

/// Best-effort request-body MIME type: the request's own `Content-Type`
/// header when present, else the response MIME type, else
/// `application/octet-stream` for binary fallback.
fn request_content_type(rec: &RequestRecord) -> String {
    rec.request_headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value.clone())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            if rec.mime_type.is_empty() {
                "application/octet-stream".to_string()
            } else {
                rec.mime_type.clone()
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::redact::REDACTED;

    fn sample_record() -> RequestRecord {
        RequestRecord {
            request_id: "req-1".to_string(),
            url: "http://example.com/".to_string(),
            method: "GET".to_string(),
            resource_type: "Document".to_string(),
            request_headers: vec![("User-Agent".to_string(), "TestUA/1.0".to_string())],
            post_body: None,
            post_body_truncated: false,
            status: Some(200),
            response_headers: vec![("content-type".to_string(), "text/html".to_string())],
            mime_type: "text/html".to_string(),
            started_at_ms: 1_000_000.0,
            finished_at_ms: Some(1_000_042.0),
            from_cache: false,
            response_body_length: Some(512),
        }
    }

    #[test]
    fn to_har_json_produces_log_with_entries() {
        let records = vec![sample_record(), sample_record()];
        let har = to_har_json(&records);
        let log = &har["log"];
        assert_eq!(log["version"], "1.2");
        assert_eq!(log["creator"]["name"], "oxibrowser");
        assert_eq!(log["entries"].as_array().unwrap().len(), 2);

        let entry = &log["entries"][0];
        assert_eq!(entry["request"]["method"], "GET");
        assert_eq!(entry["request"]["url"], "http://example.com/");
        assert_eq!(entry["request"]["bodySize"], SIZE_UNKNOWN);
        assert_eq!(entry["response"]["status"], 200);
        assert_eq!(entry["response"]["content"]["size"], 512);
        assert_eq!(entry["response"]["content"]["mimeType"], "text/html");
        assert_eq!(entry["time"], 42.0);
        // ISO 8601 with millis and Z offset.
        assert!(entry["startedDateTime"].as_str().unwrap().ends_with('Z'));
    }

    #[test]
    fn to_har_json_redacts_non_form_post_body_and_raw_keeps_it() {
        let mut rec = sample_record();
        rec.method = "POST".to_string();
        rec.post_body = Some(vec![0x00, 0xff, 0x10]);
        rec.post_body_truncated = false;
        // Default export: binary body is opaque → fixed REDACTED marker.
        let har = to_har_json(&[rec.clone()]);
        let entry = &har["log"]["entries"][0];
        assert_eq!(entry["request"]["postData"]["text"], REDACTED);
        assert_eq!(entry["request"]["bodySize"], 3);
        // Raw export (`--har-raw`): verbatim base64 body.
        let raw = to_har_json_raw(&[rec]);
        let raw_entry = &raw["log"]["entries"][0];
        assert_eq!(raw_entry["request"]["postData"]["text"], "AP8Q");
        assert_eq!(raw_entry["request"]["postData"]["encoding"], "base64");
    }

    #[test]
    fn to_har_json_redacts_sensitive_headers_and_query() {
        let mut rec = sample_record();
        rec.url = "https://example.com/cb?code=oauth-code&x=1".to_string();
        rec.request_headers = vec![
            ("Authorization".to_string(), "Bearer abc".to_string()),
            ("Accept".to_string(), "text/html".to_string()),
        ];
        rec.response_headers = vec![
            ("set-cookie".to_string(), "sid=secret".to_string()),
            ("content-type".to_string(), "text/html".to_string()),
        ];
        let har = to_har_json(&[rec.clone()]);
        let entry = &har["log"]["entries"][0];
        assert_eq!(entry["request"]["headers"][0]["value"], REDACTED);
        assert_eq!(entry["request"]["headers"][1]["value"], "text/html");
        assert_eq!(entry["response"]["headers"][0]["value"], REDACTED);
        let url = entry["request"]["url"].as_str().unwrap();
        assert!(url.contains(&format!("code={REDACTED}")), "{url}");
        assert!(url.contains("x=1"), "{url}");
        // Raw export leaves everything verbatim.
        let raw = to_har_json_raw(&[rec]);
        let raw_entry = &raw["log"]["entries"][0];
        assert_eq!(raw_entry["request"]["headers"][0]["value"], "Bearer abc");
        assert_eq!(raw_entry["response"]["headers"][0]["value"], "sid=secret");
        assert_eq!(
            raw_entry["request"]["url"].as_str().unwrap(),
            "https://example.com/cb?code=oauth-code&x=1"
        );
    }

    #[test]
    fn to_har_json_field_redacts_form_post_body() {
        let mut rec = sample_record();
        rec.method = "POST".to_string();
        rec.request_headers = vec![(
            "Content-Type".to_string(),
            "application/x-www-form-urlencoded".to_string(),
        )];
        rec.post_body = Some(b"user=a&password=hunter2".to_vec());
        let har = to_har_json(&[rec]);
        let entry = &har["log"]["entries"][0];
        let text = entry["request"]["postData"]["text"].as_str().unwrap();
        // Form bodies are field-redacted (values replaced) then base64'd.
        let decoded = String::from_utf8(
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, text).unwrap(),
        )
        .unwrap();
        assert!(decoded.contains("user=a"), "{decoded}");
        assert!(
            decoded.contains(&format!("password={REDACTED}")),
            "{decoded}"
        );
        assert!(!decoded.contains("hunter2"), "{decoded}");
    }

    #[test]
    fn to_har_json_handles_unfinished_and_binary_records() {
        let mut rec = sample_record();
        rec.status = None;
        rec.finished_at_ms = None;
        rec.response_body_length = None;
        rec.mime_type = String::new();
        rec.post_body = Some(vec![0xde, 0xad]);
        rec.post_body_truncated = true;
        let har = to_har_json(&[rec]);
        let entry = &har["log"]["entries"][0];
        assert_eq!(entry["response"]["status"], 0);
        assert_eq!(entry["response"]["content"]["size"], SIZE_UNKNOWN);
        assert_eq!(entry["time"], 0.0);
    }
}
