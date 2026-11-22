use chrono::{DateTime, Utc};
use reqwest::{Client, Method, RequestBuilder};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;
use std::time::Duration;
use thiserror::Error;
use url::Url;
use uuid::Uuid;

const MAX_AUDIT_ENTRIES: usize = 500;
const USER_AGENT: &str = "Cinder/3.1 (Local-First)";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyMode {
    StrictLocal,
    Hybrid,
}

impl Default for PrivacyMode {
    fn default() -> Self {
        Self::StrictLocal
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditOutcome {
    AllowedLoopback,
    AllowedExternal,
    BlockedByStrictLocal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub method: String,
    pub url: String,
    pub host: String,
    pub purpose: String,
    pub outcome: AuditOutcome,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditSummary {
    pub privacy_mode: PrivacyMode,
    pub total_loopback_requests: u64,
    pub external_requests_allowed: u64,
    pub external_requests_blocked: u64,
    pub entries: Vec<AuditEntry>,
}

#[derive(Debug, Error)]
pub enum NetError {
    #[error("Invalid URL '{0}': {1}")]
    InvalidUrl(String, String),
    #[error("Unsupported URL scheme '{0}'")]
    UnsupportedScheme(String),
    #[error("Strict Local Mode blocked outbound {method} request to external host '{host}' (purpose: {purpose}). Only the user can enable Hybrid Mode in Cinder's sidebar or Settings.")]
    BlockedByStrictLocal {
        method: String,
        host: String,
        purpose: String,
    },
}

pub struct NetworkGate {
    mode: RwLock<PrivacyMode>,
    entries: RwLock<VecDeque<AuditEntry>>,
    loopback_allowed: AtomicU64,
    external_allowed: AtomicU64,
    external_blocked: AtomicU64,
    api_client: Client,
    download_client: Client,
}

impl NetworkGate {
    pub fn new(initial_mode: PrivacyMode) -> anyhow::Result<Self> {
        let api_client = Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(180))
            .no_proxy()
            .build()?;

        let download_client = Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(20))
            .no_proxy()
            .build()?;

        Ok(Self {
            mode: RwLock::new(initial_mode),
            entries: RwLock::new(VecDeque::with_capacity(MAX_AUDIT_ENTRIES)),
            loopback_allowed: AtomicU64::new(0),
            external_allowed: AtomicU64::new(0),
            external_blocked: AtomicU64::new(0),
            api_client,
            download_client,
        })
    }

    pub fn mode(&self) -> PrivacyMode {
        *self.mode.read().expect("NetworkGate mode lock poisoned")
    }

    pub fn set_mode(&self, new_mode: PrivacyMode) {
        let mut guard = self.mode.write().expect("NetworkGate mode lock poisoned");
        *guard = new_mode;
    }

    pub fn classify_url(raw_url: &str) -> Result<(Url, String, bool), NetError> {
        let parsed = Url::parse(raw_url)
            .map_err(|e| NetError::InvalidUrl(raw_url.to_string(), e.to_string()))?;

        match parsed.scheme() {
            "http" | "https" => {}
            other => return Err(NetError::UnsupportedScheme(other.to_string())),
        }

        let host = parsed
            .host_str()
            .ok_or_else(|| NetError::InvalidUrl(raw_url.to_string(), "missing host".into()))?
            .to_string();

        let is_loopback = is_loopback_host(&host);
        Ok((parsed, host, is_loopback))
    }

    /// Authorize an outbound HTTP request and construct a RequestBuilder using the API client.
    pub fn request(
        &self,
        method: Method,
        raw_url: &str,
        purpose: &str,
    ) -> Result<RequestBuilder, NetError> {
        self.authorize_with_client(&self.api_client, method, raw_url, purpose)
    }

    /// Authorize an outbound HTTP request for large streaming downloads (no total timeout).
    pub fn download_request(
        &self,
        method: Method,
        raw_url: &str,
        purpose: &str,
    ) -> Result<RequestBuilder, NetError> {
        self.authorize_with_client(&self.download_client, method, raw_url, purpose)
    }

    fn authorize_with_client(
        &self,
        client: &Client,
        method: Method,
        raw_url: &str,
        purpose: &str,
    ) -> Result<RequestBuilder, NetError> {
        let (parsed, host, is_loopback) = Self::classify_url(raw_url)?;
        let current_mode = self.mode();

        let outcome = if is_loopback {
            self.loopback_allowed.fetch_add(1, Ordering::Relaxed);
            AuditOutcome::AllowedLoopback
        } else if current_mode == PrivacyMode::StrictLocal {
            self.external_blocked.fetch_add(1, Ordering::Relaxed);
            self.record_entry(&method, &parsed, &host, purpose, AuditOutcome::BlockedByStrictLocal);
            return Err(NetError::BlockedByStrictLocal {
                method: method.to_string(),
                host,
                purpose: purpose.to_string(),
            });
        } else {
            self.external_allowed.fetch_add(1, Ordering::Relaxed);
            AuditOutcome::AllowedExternal
        };

        self.record_entry(&method, &parsed, &host, purpose, outcome);
        Ok(client
            .request(method, parsed)
            .header("User-Agent", USER_AGENT))
    }

    fn record_entry(
        &self,
        method: &Method,
        url: &Url,
        host: &str,
        purpose: &str,
        outcome: AuditOutcome,
    ) {
        let sanitized_url = sanitize_url_for_audit(url);
        let entry = AuditEntry {
            id: Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            method: method.to_string(),
            url: sanitized_url,
            host: host.to_string(),
            purpose: purpose.to_string(),
            outcome,
        };
        let mut guard = self.entries.write().expect("NetworkGate entries lock poisoned");
        if guard.len() >= MAX_AUDIT_ENTRIES {
            guard.pop_front();
        }
        guard.push_back(entry);
    }

    pub fn audit_summary(&self) -> AuditSummary {
        let guard = self.entries.read().expect("NetworkGate entries lock poisoned");
        let entries: Vec<AuditEntry> = guard.iter().rev().cloned().collect();
        AuditSummary {
            privacy_mode: self.mode(),
            total_loopback_requests: self.loopback_allowed.load(Ordering::Relaxed),
            external_requests_allowed: self.external_allowed.load(Ordering::Relaxed),
            external_requests_blocked: self.external_blocked.load(Ordering::Relaxed),
            entries,
        }
    }

    pub fn clear_audit(&self) {
        let mut guard = self.entries.write().expect("NetworkGate entries lock poisoned");
        guard.clear();
        self.loopback_allowed.store(0, Ordering::Relaxed);
        self.external_allowed.store(0, Ordering::Relaxed);
        self.external_blocked.store(0, Ordering::Relaxed);
    }
}

pub fn is_loopback_host(host: &str) -> bool {
    let trimmed = host.trim().trim_start_matches('[').trim_end_matches(']');
    if trimmed.eq_ignore_ascii_case("localhost") {
        return true;
    }
    if let Ok(ip) = trimmed.parse::<IpAddr>() {
        return ip.is_loopback();
    }
    false
}

/// Strip sensitive query tokens (like API keys) before writing audit logs.
fn sanitize_url_for_audit(url: &Url) -> String {
    let mut cleaned = url.clone();
    if url.query().is_some() {
        let safe_pairs: Vec<(String, String)> = url
            .query_pairs()
            .map(|(k, v)| {
                let lower = k.to_ascii_lowercase();
                if lower.contains("key") || lower.contains("token") || lower.contains("secret") {
                    (k.into_owned(), "[REDACTED]".to_string())
                } else {
                    (k.into_owned(), v.into_owned())
                }
            })
            .collect();
        cleaned.query_pairs_mut().clear().extend_pairs(safe_pairs);
    }
    cleaned.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_local_allows_loopback_and_blocks_external() {
        let gate = NetworkGate::new(PrivacyMode::StrictLocal).unwrap();

        assert!(gate
            .request(Method::POST, "http://127.0.0.1:18080/v1/chat/completions", "local_inference")
            .is_ok());
        assert!(gate
            .request(Method::GET, "http://localhost:11434/api/tags", "local_ollama")
            .is_ok());
        assert!(gate
            .request(Method::GET, "http://[::1]:18080/health", "local_health")
            .is_ok());

        let blocked = gate.request(
            Method::GET,
            "https://huggingface.co/api/models?search=qwen",
            "hf_hub_search",
        );
        assert!(matches!(blocked, Err(NetError::BlockedByStrictLocal { .. })));

        let summary = gate.audit_summary();
        assert_eq!(summary.total_loopback_requests, 3);
        assert_eq!(summary.external_requests_allowed, 0);
        assert_eq!(summary.external_requests_blocked, 1);
    }

    #[test]
    fn hybrid_allows_external_and_redacts_keys_in_audit() {
        let gate = NetworkGate::new(PrivacyMode::Hybrid).unwrap();
        let req = gate.request(
            Method::POST,
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-pro:generateContent?key=supersecret123",
            "gemini_api",
        );
        assert!(req.is_ok());

        let summary = gate.audit_summary();
        assert_eq!(summary.external_requests_allowed, 1);
        assert_eq!(summary.external_requests_blocked, 0);
        assert!(summary.entries[0].url.contains("REDACTED"));
        assert!(!summary.entries[0].url.contains("supersecret123"));
    }
}
