//! Quota normalization and source selection; no credentials or IO.

use crate::usage::{Provider, ProviderUsage, UsageSource, WindowState, WindowUsage};
use gtk4::glib;
use serde_json::{json, Value};
use std::time::Duration;

pub const REFRESH_INTERVAL: Duration = Duration::from_secs(60);
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const FRESH_FOR_MS: u64 = 5 * 60 * 1000;
const FUTURE_TOLERANCE_MS: u64 = 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeError {
    Unavailable,
    Timeout,
    Protocol,
    Authentication,
    RateLimited,
    Network,
}

impl ProbeError {
    pub fn problem(self) -> &'static str {
        match self {
            Self::Unavailable => "direct-unavailable",
            Self::Timeout => "direct-timeout",
            Self::Protocol => "direct-schema",
            Self::Authentication => "direct-auth",
            Self::RateLimited => "direct-rate-limited",
            Self::Network => "direct-network",
        }
    }
}

pub fn initialize_request() -> Value {
    json!({"id":1,"method":"initialize","params":{
        "clientInfo":{"name":"iskra_usage","version":"0.1"},
        "capabilities":{"experimentalApi":false}
    }})
}

pub fn initialized_notification() -> Value {
    json!({"method":"initialized","params":{}})
}

pub fn limits_request() -> Value {
    // No model turn, credit redemption, or experiment exposure.
    json!({"id":2,"method":"account/rateLimits/read","params":{
        "excludeResetCreditDetails":true,"supportsLunaReserve":false
    }})
}

pub fn rpc_error(error: &Value) -> ProbeError {
    if matches!(
        error.get("code").and_then(Value::as_i64),
        Some(-32601 | -32602)
    ) {
        return ProbeError::Protocol;
    }
    let message = error.get("message").and_then(Value::as_str).unwrap_or("");
    let lower = message.to_ascii_lowercase();
    if lower.contains("401") || lower.contains("authentication required") {
        ProbeError::Authentication
    } else if lower.contains("429") {
        ProbeError::RateLimited
    } else {
        ProbeError::Network
    }
}

fn blank() -> ProviderUsage {
    ProviderUsage {
        provider: Provider::Codex,
        source: UsageSource::CodexDirect,
        windows: ["5h", "7d"].map(|label| WindowUsage {
            label,
            used_fraction: None,
            resets_at: None,
            state: WindowState::Unavailable,
        }),
        fetched_at_ms: None,
        problem: Some("direct-schema"),
    }
}

/// Select the main Codex bucket, never a model-specific or reserve bucket.
fn main_bucket(result: &Value) -> Option<&Value> {
    if let Some(map) = result.get("rateLimitsByLimitId").filter(|v| !v.is_null()) {
        let bucket = map.as_object()?.get("codex").filter(|v| v.is_object())?;
        return match bucket.get("limitId") {
            None | Some(Value::Null) => Some(bucket),
            Some(Value::String(id)) if id == "codex" => Some(bucket),
            _ => None,
        };
    }
    let legacy = result.get("rateLimits").filter(|v| v.is_object())?;
    match legacy.get("limitId") {
        None | Some(Value::Null) => Some(legacy),
        Some(Value::String(id)) if id == "codex" => Some(legacy),
        _ => None,
    }
}

/// Re-evaluate age/reset on each cache tick, not only at network refresh.
pub fn normalize(
    result: &Value,
    fetched_at_ms: u64,
    now_ms: u64,
    failure: Option<ProbeError>,
) -> ProviderUsage {
    let mut out = blank();
    if fetched_at_ms == 0 || fetched_at_ms > now_ms.saturating_add(FUTURE_TOLERANCE_MS) {
        return out;
    }
    out.fetched_at_ms = Some(fetched_at_ms);
    let Some(bucket) = main_bucket(result) else {
        return out;
    };
    let stale = failure.is_some() || now_ms.saturating_sub(fetched_at_ms) > FRESH_FOR_MS;
    let mut seen = [false; 2];
    for name in ["primary", "secondary"] {
        let Some(w) = bucket.get(name).filter(|w| w.is_object()) else {
            continue;
        };
        let i = match w.get("windowDurationMins").and_then(Value::as_i64) {
            Some(300) => 0,
            Some(10080) => 1,
            _ => continue,
        };
        if seen[i] {
            out.windows[i].used_fraction = None;
            out.windows[i].state = WindowState::Unavailable;
            continue;
        }
        seen[i] = true;
        let Some(seconds) = w.get("resetsAt").and_then(Value::as_i64).filter(|v| *v > 0) else {
            continue;
        };
        let Some(reset_ms) = u64::try_from(seconds)
            .ok()
            .and_then(|v| v.checked_mul(1000))
        else {
            continue;
        };
        let Some(reset_text) = glib::DateTime::from_unix_utc(seconds)
            .ok()
            .and_then(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").ok())
        else {
            continue;
        };
        let row = &mut out.windows[i];
        row.resets_at = Some(reset_text.to_string());
        if reset_ms <= now_ms {
            row.state = WindowState::ResetPending;
            continue;
        }
        let Some(percent) = w
            .get("usedPercent")
            .and_then(Value::as_u64)
            .filter(|v| *v <= 100)
        else {
            continue;
        };
        row.used_fraction = Some(percent as f64 / 100.0);
        row.state = if stale {
            WindowState::Stale
        } else {
            WindowState::Live
        };
    }
    out.problem = failure.map(ProbeError::problem).or_else(|| {
        if out
            .windows
            .iter()
            .any(|w| w.state == WindowState::Unavailable)
        {
            Some("direct-schema")
        } else if stale {
            Some("stale")
        } else {
            None
        }
    });
    out
}

pub fn failed(error: ProbeError) -> ProviderUsage {
    let mut out = blank();
    out.problem = Some(error.problem());
    out
}

pub fn needs_fallback(cache: &ProviderUsage) -> bool {
    cache.provider == Provider::Codex
        && cache.problem != Some("multiple-accounts")
        && !cache
            .windows
            .iter()
            .all(|w| w.state == WindowState::Live && w.used_fraction.is_some())
}

fn quality(p: &ProviderUsage) -> (usize, usize, u64) {
    (
        p.windows
            .iter()
            .filter(|w| w.state == WindowState::Live && w.used_fraction.is_some())
            .count(),
        p.windows
            .iter()
            .filter(|w| w.used_fraction.is_some())
            .count(),
        p.fetched_at_ms.unwrap_or(0),
    )
}

/// Choose a whole provider snapshot; never splice windows/accounts/sources.
pub fn choose(cache: ProviderUsage, direct: Option<ProviderUsage>) -> ProviderUsage {
    if !needs_fallback(&cache) {
        return cache;
    }
    match direct {
        Some(d) if d.provider == Provider::Codex && quality(&d) > quality(&cache) => d,
        _ => cache,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_794_445_667_041;

    fn fixture() -> Value {
        // Synthetic committed fixture, not a captured provider response.
        let frozen: Value =
            serde_json::from_str(include_str!("../tests/fixtures/codex-rate-limits.json")).unwrap();
        frozen["result"].clone()
    }

    #[test]
    fn synthetic_response_and_reversed_windows_are_exact() {
        let mut input = fixture();
        let out = normalize(&input, NOW, NOW, None);
        assert_eq!(out.source, UsageSource::CodexDirect);
        assert_eq!(out.windows[0].percent_text(), "37%");
        assert_eq!(out.windows[0].remaining_percent(), Some(63));
        assert_eq!(out.windows[1].percent_text(), "55%");
        assert_eq!(out.windows[1].remaining_percent(), Some(45));
        assert_eq!(out.problem, None);
        let a = input["rateLimitsByLimitId"]["codex"]["primary"].clone();
        input["rateLimitsByLimitId"]["codex"]["primary"] =
            input["rateLimitsByLimitId"]["codex"]["secondary"].clone();
        input["rateLimitsByLimitId"]["codex"]["secondary"] = a;
        let reversed = normalize(&input, NOW, NOW, None);
        assert_eq!(reversed, out);
    }

    #[test]
    fn zero_missing_and_invalid_percent_are_distinct() {
        for (value, expected) in [
            (json!(0), "0%"),
            (json!(100), "100%"),
            (Value::Null, "--"),
            (json!(-1), "--"),
            (json!(101), "--"),
            (json!("37"), "--"),
            (json!(37.5), "--"),
            (json!(false), "--"),
        ] {
            let mut input = fixture();
            input["rateLimitsByLimitId"]["codex"]["primary"]["usedPercent"] = value;
            let out = normalize(&input, NOW, NOW, None);
            assert_eq!(out.windows[0].percent_text(), expected);
            assert_eq!(out.windows[1].percent_text(), "55%");
        }
    }

    #[test]
    fn unknown_durations_and_ambiguous_windows_do_not_use_order() {
        let mut input = fixture();
        input["rateLimitsByLimitId"]["codex"]["primary"]["windowDurationMins"] = Value::Null;
        let out = normalize(&input, NOW, NOW, None);
        assert_eq!(out.windows[0].percent_text(), "--");
        assert_eq!(out.windows[1].percent_text(), "55%");
        input["rateLimitsByLimitId"]["codex"]["primary"]["windowDurationMins"] = json!(10080);
        let out = normalize(&input, NOW, NOW, None);
        assert!(out.windows.iter().all(|w| w.used_fraction.is_none()));
    }

    #[test]
    fn never_substitutes_a_model_bucket_or_infers_global_recovery() {
        let mut contradictory = fixture();
        contradictory["rateLimitsByLimitId"]["codex"]["limitId"] = json!("reserve");
        assert!(normalize(&contradictory, NOW, NOW, None)
            .windows
            .iter()
            .all(|w| w.used_fraction.is_none()));
        let mut input = fixture();
        let model = input["rateLimitsByLimitId"]["codex"].take();
        input["rateLimitsByLimitId"] = json!({"reserve":model});
        let out = normalize(&input, NOW, NOW, None);
        assert!(out.windows.iter().all(|w| w.used_fraction.is_none()));
        // Legacy single-bucket responses are accepted only for main Codex.
        input["rateLimitsByLimitId"] = Value::Null;
        assert_eq!(
            normalize(&input, NOW, NOW, None).windows[0].percent_text(),
            "37%"
        );
        input["rateLimits"]["limitId"] = json!("reserve");
        assert!(normalize(&input, NOW, NOW, None).windows[0]
            .used_fraction
            .is_none());
        let mut input = fixture();
        input["ordinaryUsageAllowed"] = json!(false);
        // Numeric quota is not a claim that ordinary generation is allowed.
        assert_eq!(
            normalize(&input, NOW, NOW, None).windows[0].percent_text(),
            "37%"
        );
    }

    #[test]
    fn timeouts_staleness_reset_and_bad_times_keep_honest_semantics() {
        let input = fixture();
        let failed = normalize(&input, NOW, NOW, Some(ProbeError::Timeout));
        assert_eq!(failed.windows[0].percent_text(), "~37%");
        assert_eq!(failed.problem, Some("direct-timeout"));
        let old = normalize(&input, NOW, NOW + FRESH_FOR_MS + 1, None);
        assert_eq!(old.windows[1].percent_text(), "~55%");
        let reset = normalize(&input, NOW, 1_794_458_936_000, None);
        assert_eq!(reset.windows[0].state, WindowState::ResetPending);
        assert_eq!(reset.windows[0].percent_text(), "--");
        for t in [0, NOW + FUTURE_TOLERANCE_MS + 1] {
            assert!(normalize(&input, t, NOW, None)
                .windows
                .iter()
                .all(|w| w.used_fraction.is_none()));
        }
        for t in [Value::Null, json!(-1), json!(i64::MAX), json!("1794458936")] {
            let mut bad = input.clone();
            bad["rateLimitsByLimitId"]["codex"]["primary"]["resetsAt"] = t;
            assert_eq!(
                normalize(&bad, NOW, NOW, None).windows[0].percent_text(),
                "--"
            );
        }
    }

    #[test]
    fn source_selection_keeps_whole_snapshots_and_prefers_live_zeron() {
        let direct = normalize(&fixture(), NOW, NOW, None);
        let mut cache = direct.clone();
        cache.source = UsageSource::ZeronCache;
        cache.windows[0].used_fraction = Some(0.2);
        assert!(!needs_fallback(&cache));
        assert_eq!(choose(cache.clone(), Some(direct.clone())), cache);
        cache.windows[0].state = WindowState::Unavailable;
        cache.windows[0].used_fraction = None;
        assert!(needs_fallback(&cache));
        assert_eq!(choose(cache.clone(), Some(direct.clone())), direct);
        let mut stale = direct.clone();
        for w in &mut stale.windows {
            w.state = WindowState::Stale;
        }
        assert_eq!(choose(cache.clone(), Some(stale)), cache);
        cache.problem = Some("multiple-accounts");
        assert!(!needs_fallback(&cache));
        assert_eq!(choose(cache.clone(), Some(direct)), cache);
    }

    #[test]
    fn request_contract_is_read_only_and_no_raw_errors_escape() {
        assert_eq!(limits_request()["method"], "account/rateLimits/read");
        assert_eq!(
            limits_request()["params"]["excludeResetCreditDetails"],
            true
        );
        assert_eq!(limits_request()["params"]["supportsLunaReserve"], false);
        let error = rpc_error(&json!({"message":"401 token=private"}));
        assert_eq!(error, ProbeError::Authentication);
        assert_eq!(failed(error).problem, Some("direct-auth"));
        assert_eq!(rpc_error(&json!({"code":-32601})), ProbeError::Protocol);
    }
}
