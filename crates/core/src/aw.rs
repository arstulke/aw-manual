//! Minimal async client for the aw-server REST API (works with aw-server and aw-server-rust).

use std::time::Duration;

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

pub const CLIENT_NAME: &str = "aw-manual";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AwError {
    /// The server could not be reached (retry later).
    Network(String),
    /// The server failed temporarily (5xx, retry later).
    Server(u16, String),
    /// The bucket doesn't exist (create it and retry).
    BucketMissing,
    /// The server rejected the request (4xx); retrying won't help.
    Rejected(u16, String),
}

impl AwError {
    pub fn is_retryable(&self) -> bool {
        matches!(self, AwError::Network(_) | AwError::Server(..))
    }
}

impl std::fmt::Display for AwError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AwError::Network(e) => write!(f, "aw-server unreachable: {e}"),
            AwError::Server(s, b) => write!(f, "aw-server error {s}: {}", truncate(b)),
            AwError::BucketMissing => write!(f, "bucket does not exist"),
            AwError::Rejected(s, b) => {
                write!(f, "aw-server rejected the request ({s}): {}", truncate(b))
            }
        }
    }
}

impl std::error::Error for AwError {}

fn truncate(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() > 200 {
        format!("{}…", s.chars().take(200).collect::<String>())
    } else {
        s.to_string()
    }
}

/// An event as returned by aw-server.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AwEvent {
    pub id: Option<i64>,
    pub timestamp: DateTime<Utc>,
    pub duration: f64,
    pub data: serde_json::Value,
}

impl AwEvent {
    pub fn end(&self) -> DateTime<Utc> {
        self.timestamp + secs(self.duration)
    }
}

/// An event to be inserted.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NewEvent {
    #[serde(serialize_with = "ser_ts")]
    pub timestamp: DateTime<Utc>,
    pub duration: f64,
    pub data: serde_json::Value,
}

fn ser_ts<S: serde::Serializer>(t: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&fmt_ts(*t))
}

pub fn fmt_ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Converts seconds (as used by aw-server) into a chrono duration.
pub fn secs(s: f64) -> chrono::Duration {
    chrono::Duration::milliseconds((s * 1000.0).round() as i64)
}

/// Converts a chrono duration into seconds.
pub fn to_secs(d: chrono::Duration) -> f64 {
    d.num_milliseconds() as f64 / 1000.0
}

fn encode_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[derive(Debug, Clone)]
pub struct AwClient {
    http: reqwest::Client,
    base: String,
}

impl AwClient {
    pub fn new(base_url: &str) -> Self {
        let base = base_url.trim().trim_end_matches('/').to_string();
        let mut builder = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(15))
            // Requests are rare (one heartbeat per minute); not reusing
            // connections avoids races with servers closing idle ones.
            .pool_max_idle_per_host(0)
            .user_agent(concat!("aw-manual/", env!("CARGO_PKG_VERSION")));
        if is_loopback(&base) {
            builder = builder.no_proxy();
        }
        AwClient {
            http: builder.build().expect("HTTP client can be built"),
            base,
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    fn bucket_url(&self, bucket: &str) -> String {
        format!("{}/api/0/buckets/{}", self.base, encode_segment(bucket))
    }

    async fn send(&self, req: reqwest::RequestBuilder) -> Result<reqwest::Response, AwError> {
        let resp = req
            .send()
            .await
            .map_err(|e| AwError::Network(error_chain(&e)))?;
        let status = resp.status().as_u16();
        if resp.status().is_success() || status == 304 {
            return Ok(resp);
        }
        let body = resp.text().await.unwrap_or_default();
        Err(match status {
            404 => AwError::BucketMissing,
            500..=599 => AwError::Server(status, body),
            _ => AwError::Rejected(status, body),
        })
    }

    /// `GET /api/0/info` – used to check that the server is reachable.
    pub async fn info(&self) -> Result<serde_json::Value, AwError> {
        let resp = self
            .send(self.http.get(format!("{}/api/0/info", self.base)))
            .await?;
        resp.json()
            .await
            .map_err(|e| AwError::Network(e.to_string()))
    }

    /// Creates the bucket if it doesn't exist yet.
    pub async fn ensure_bucket(
        &self,
        bucket: &str,
        bucket_type: &str,
        hostname: &str,
    ) -> Result<(), AwError> {
        let body =
            serde_json::json!({ "client": CLIENT_NAME, "type": bucket_type, "hostname": hostname });
        self.send(self.http.post(self.bucket_url(bucket)).json(&body))
            .await
            .map(|_| ())
    }

    pub async fn heartbeat(
        &self,
        bucket: &str,
        event: &NewEvent,
        pulsetime: f64,
    ) -> Result<(), AwError> {
        let url = format!(
            "{}/heartbeat?pulsetime={}",
            self.bucket_url(bucket),
            pulsetime
        );
        self.send(self.http.post(url).json(event)).await.map(|_| ())
    }

    pub async fn insert(&self, bucket: &str, events: &[NewEvent]) -> Result<(), AwError> {
        if events.is_empty() {
            return Ok(());
        }
        let url = format!("{}/events", self.bucket_url(bucket));
        self.send(self.http.post(url).json(events))
            .await
            .map(|_| ())
    }

    /// Events overlapping `[start, end]`. Servers may clip the returned events
    /// to the range; use [`AwClient::get_event`] to get the full extent.
    pub async fn events_between(
        &self,
        bucket: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<AwEvent>, AwError> {
        let url = format!(
            "{}/events?start={}&end={}&limit=-1",
            self.bucket_url(bucket),
            encode_segment(&fmt_ts(start)),
            encode_segment(&fmt_ts(end))
        );
        let resp = self.send(self.http.get(url)).await?;
        resp.json()
            .await
            .map_err(|e| AwError::Rejected(200, format!("invalid events response: {e}")))
    }

    pub async fn get_event(&self, bucket: &str, id: i64) -> Result<Option<AwEvent>, AwError> {
        let url = format!("{}/events/{}", self.bucket_url(bucket), id);
        match self.send(self.http.get(url)).await {
            Ok(resp) => {
                let v: serde_json::Value = resp
                    .json()
                    .await
                    .map_err(|e| AwError::Network(e.to_string()))?;
                if v.is_null() {
                    return Ok(None);
                }
                serde_json::from_value(v)
                    .map(Some)
                    .map_err(|e| AwError::Rejected(200, format!("invalid event: {e}")))
            }
            Err(AwError::BucketMissing) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub async fn delete_event(&self, bucket: &str, id: i64) -> Result<(), AwError> {
        let url = format!("{}/events/{}", self.bucket_url(bucket), id);
        match self.send(self.http.delete(url)).await {
            Ok(_) | Err(AwError::BucketMissing) => Ok(()),
            Err(e) => Err(e),
        }
    }
}

fn error_chain(e: &dyn std::error::Error) -> String {
    let mut s = e.to_string();
    let mut src = e.source();
    while let Some(inner) = src {
        let t = inner.to_string();
        if !s.contains(&t) {
            s.push_str(": ");
            s.push_str(&t);
        }
        src = inner.source();
    }
    s
}

fn is_loopback(url: &str) -> bool {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let host_port = rest.split('/').next().unwrap_or_default();
    let host = if host_port.starts_with('[') {
        host_port
            .split(']')
            .next()
            .unwrap_or_default()
            .trim_start_matches('[')
    } else {
        host_port.split(':').next().unwrap_or_default()
    };
    host.eq_ignore_ascii_case("localhost") || host.starts_with("127.") || host == "::1"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_detection() {
        assert!(is_loopback("http://localhost:5600"));
        assert!(is_loopback("http://127.0.0.1:5600/"));
        assert!(is_loopback("http://[::1]:5600"));
        assert!(!is_loopback("https://aw.example.com"));
    }

    #[test]
    fn segment_encoding() {
        assert_eq!(
            encode_segment("aw-manual_host.local"),
            "aw-manual_host.local"
        );
        assert_eq!(encode_segment("a b/c+"), "a%20b%2Fc%2B");
    }

    #[test]
    fn timestamps() {
        let t: DateTime<Utc> = "2026-10-09T08:30:00.123456Z".parse().unwrap();
        assert_eq!(fmt_ts(t), "2026-10-09T08:30:00.123Z");
        assert_eq!(to_secs(secs(1.5)), 1.5);
    }
}
