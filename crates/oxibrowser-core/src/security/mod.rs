//! Security plumbing shared across observation and persistence surfaces:
//! secret redaction for network logs / HAR, and the append-only audit log.

pub mod audit;
pub mod redact;
