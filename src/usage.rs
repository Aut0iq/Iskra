//! Read-only subscription measurements from Zeron's normalized usage cache.
use gtk4::glib;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAX_CACHE_BYTES: u64 = 1024 * 1024;
const FRESH_FOR_MS: u64 = 5 * 60 * 1000;
const FUTURE_TOLERANCE_MS: u64 = 60 * 1000;
const POLL_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Devin,
    Codex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowState {
    Live,
    Stale,
    Unavailable,
    ResetPending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageSource {
    ZeronCache,
    CodexDirect,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WindowUsage {
    pub label: &'static str,
    pub used_fraction: Option<f64>,
    pub resets_at: Option<String>,
    pub state: WindowState,
}

impl WindowUsage {
    pub fn used_percent(&self) -> Option<u8> {
        self.used_fraction.map(|v| (v * 100.0).round() as u8)
    }

    pub fn remaining_percent(&self) -> Option<u8> {
        self.used_percent().map(|v| 100 - v)
    }

    pub fn percent_text(&self) -> String {
        match self.used_percent() {
            Some(v) if self.state == WindowState::Stale => format!("~{v}%"),
            Some(v) => format!("{v}%"),
            None => "--".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderUsage {
    pub provider: Provider,
    pub source: UsageSource,
    pub windows: [WindowUsage; 2],
    pub fetched_at_ms: Option<u64>,
    /// Allowlisted reason only; cache error payloads and credentials never escape.
    pub problem: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UsageSnapshot {
    pub providers: [ProviderUsage; 2],
}

impl UsageSnapshot {
    pub fn unavailable(problem: &'static str) -> Self {
        Self {
            providers: [
                empty_provider(Provider::Devin, problem),
                empty_provider(Provider::Codex, problem),
            ],
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct Cache {
    entries: BTreeMap<String, Entry>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    usage: Option<CachedUsage>,
    fetched_at: Option<u64>,
    error: Option<CachedError>,
}

#[derive(Debug, Clone, Deserialize)]
struct CachedUsage {
    windows: Vec<CachedWindow>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedWindow {
    label: String,
    used_fraction: Option<f64>,
    resets_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct CachedError {
    kind: Option<String>,
}

fn empty_provider(provider: Provider, problem: &'static str) -> ProviderUsage {
    let labels = match provider {
        Provider::Devin => ["24h", "7d"],
        Provider::Codex => ["5h", "7d"],
    };
    ProviderUsage {
        provider,
        source: UsageSource::ZeronCache,
        windows: labels.map(|label| WindowUsage {
            label,
            used_fraction: None,
            resets_at: None,
            state: WindowState::Unavailable,
        }),
        fetched_at_ms: None,
        problem: Some(problem),
    }
}

fn window_index(provider: Provider, label: &str) -> Option<usize> {
    match label.trim().to_ascii_lowercase().as_str() {
        "week" | "weekly" | "7d" | "7-day" => Some(1),
        "day" | "daily" | "24h" if provider == Provider::Devin => Some(0),
        "session" | "5h" | "5-hour" | "5 hours" if provider == Provider::Codex => Some(0),
        _ => None,
    }
}

fn reset_millis(text: &str) -> Option<u64> {
    // A missing timezone must not silently use the machine's local timezone.
    let time = text.split_once('T')?.1;
    if !time.ends_with('Z') && !time.contains('+') && !time.contains('-') {
        return None;
    }
    let dt = glib::DateTime::from_iso8601(text, None).ok()?;
    u64::try_from(dt.to_unix()).ok()?.checked_mul(1000)
}

fn provider_usage(cache: &Cache, provider: Provider, now_ms: u64) -> ProviderUsage {
    let prefix = match provider {
        Provider::Devin => "devin:",
        Provider::Codex => "codex:",
    };
    let mut matching = cache
        .entries
        .iter()
        .filter(|(key, _)| key.starts_with(prefix));
    let Some((_, entry)) = matching.next() else {
        return empty_provider(provider, "missing");
    };
    if matching.next().is_some() {
        return empty_provider(provider, "multiple-accounts");
    }
    let mut out = empty_provider(provider, "missing-windows");
    out.fetched_at_ms = entry.fetched_at;
    out.problem = entry.error.as_ref().map(|e| match e.kind.as_deref() {
        Some("no-credentials") => "no-credentials",
        Some("unauthorized") => "unauthorized",
        Some("network") => "network",
        _ => "provider-error",
    });
    let Some(fetched) = entry
        .fetched_at
        .filter(|&v| v > 0 && v <= now_ms.saturating_add(FUTURE_TOLERANCE_MS))
    else {
        out.problem = out.problem.or(Some("missing-timestamp"));
        return out;
    };
    let Some(usage) = &entry.usage else {
        out.problem = out.problem.or(Some("missing"));
        return out;
    };
    let stale = entry.error.is_some() || now_ms.saturating_sub(fetched) > FRESH_FOR_MS;
    let mut seen = [false; 2];
    for window in &usage.windows {
        let Some(i) = window_index(provider, &window.label) else {
            continue;
        };
        if seen[i] {
            // Two measurements for the same named window are ambiguous.
            out.windows[i].used_fraction = None;
            out.windows[i].state = WindowState::Unavailable;
            continue;
        }
        seen[i] = true;
        let row = &mut out.windows[i];
        row.resets_at = window.resets_at.clone();
        let Some(reset) = window.resets_at.as_deref().and_then(reset_millis) else {
            continue;
        };
        if reset <= now_ms {
            row.state = WindowState::ResetPending;
            continue;
        }
        let Some(fraction) = window
            .used_fraction
            .filter(|v| v.is_finite() && (0.0..=1.0).contains(v))
        else {
            continue;
        };
        row.used_fraction = Some(fraction);
        row.state = if stale {
            WindowState::Stale
        } else {
            WindowState::Live
        };
    }
    if out.problem.is_none() {
        // Recognized labels whose payloads all failed validation are a
        // data problem too — never a silent "--" row set.
        out.problem = if !seen.iter().all(|v| *v)
            || out
                .windows
                .iter()
                .all(|w| w.state == WindowState::Unavailable)
        {
            Some("missing-windows")
        } else if stale {
            Some("stale")
        } else {
            None
        };
    }
    out
}

fn snapshot(cache: &Cache, now_ms: u64) -> UsageSnapshot {
    UsageSnapshot {
        providers: [
            provider_usage(cache, Provider::Devin, now_ms),
            provider_usage(cache, Provider::Codex, now_ms),
        ],
    }
}

pub fn parse_cache(bytes: &[u8], now_ms: u64) -> Result<UsageSnapshot, serde_json::Error> {
    serde_json::from_slice(bytes).map(|cache| snapshot(&cache, now_ms))
}

pub fn cache_path() -> PathBuf {
    if let Some(p) = std::env::var_os("ISKRA_USAGE_CACHE").filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    match std::env::var_os("HOME").filter(|h| !h.is_empty()) {
        Some(home) => PathBuf::from(home).join(".zeron/agent-accounts/usage-cache.json"),
        // With no HOME a CWD-relative path could pick up an unintended
        // cache; read the guaranteed-missing one instead.
        None => PathBuf::from("/dev/null"),
    }
}

fn read_cache(path: &Path) -> Option<Cache> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_CACHE_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_CACHE_BYTES {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

pub fn spawn_cache_reader(
    path: PathBuf,
    tx: mpsc::SyncSender<UsageSnapshot>,
    stop: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    spawn_cache_reader_impl(path, tx, stop, |probe_tx, needed, probe_stop| {
        Some(crate::codex_probe::spawn_reader(
            probe_tx, needed, probe_stop,
        ))
    })
}

/// Cache loop plus the Codex-direct fallback child worker. The probe
/// spawner is injected so tests can substitute a fake or disable it
/// entirely — production always uses `codex_probe::spawn_reader`.
fn spawn_cache_reader_impl(
    path: PathBuf,
    tx: mpsc::SyncSender<UsageSnapshot>,
    stop: Arc<AtomicBool>,
    spawn_probe: impl FnOnce(
            mpsc::SyncSender<
                Result<crate::codex_probe::ProbeSample, crate::codex_limits::ProbeError>,
            >,
            Arc<AtomicBool>,
            Arc<AtomicBool>,
        ) -> Option<std::thread::JoinHandle<()>>
        + Send
        + 'static,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let needed = Arc::new(AtomicBool::new(false));
        let probe_stop = Arc::new(AtomicBool::new(false));
        let (probe_tx, probe_rx) = mpsc::sync_channel(1);
        let probe_worker = spawn_probe(probe_tx, needed.clone(), probe_stop.clone());
        let mut previous = None;
        let mut last_sample: Option<crate::codex_probe::ProbeSample> = None;
        let mut last_failure: Option<crate::codex_limits::ProbeError> = None;
        while !stop.load(Ordering::Relaxed) {
            let current = read_cache(&path);
            let failed = current.is_none();
            if let Some(cache) = current {
                previous = Some(cache);
            }
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let mut reading = previous.as_ref().map_or_else(
                || UsageSnapshot::unavailable("cache-unavailable"),
                |cache| snapshot(cache, now_ms),
            );
            if failed {
                for provider in &mut reading.providers {
                    provider.problem = Some("cache-unavailable");
                    for row in &mut provider.windows {
                        if row.state == WindowState::Live {
                            row.state = WindowState::Stale;
                        }
                    }
                }
            }
            // Direct Codex fallback only fires while Zeron's cache entry
            // is unhealthy; a live cache or ambiguous accounts keep the
            // probe worker idle. The flag is computed BEFORE choosing.
            let cache_codex = reading.providers[1].clone();
            needed.store(
                crate::codex_limits::needs_fallback(&cache_codex),
                Ordering::Relaxed,
            );
            for result in probe_rx.try_iter() {
                match result {
                    Ok(sample) => {
                        last_sample = Some(sample);
                        last_failure = None;
                    }
                    Err(error) => last_failure = Some(error),
                }
            }
            let direct = match (&last_sample, last_failure) {
                (Some(sample), failure) => Some(crate::codex_limits::normalize(
                    &sample.result,
                    sample.fetched_at_ms,
                    now_ms,
                    failure,
                )),
                (None, Some(error)) => Some(crate::codex_limits::failed(error)),
                (None, None) => None,
            };
            // Whole-provider merge only — never per-window splicing.
            reading.providers[1] = crate::codex_limits::choose(cache_codex, direct);
            if let Err(mpsc::TrySendError::Disconnected(_)) = tx.try_send(reading) {
                break;
            }
            // Shutdown is responsive without joining a sleeping worker in GTK.
            for _ in 0..(POLL_INTERVAL.as_millis() / 100) {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        // The probe worker must never outlive this loop (channel drop
        // alone would leave it polling while needed stays true).
        probe_stop.store(true, Ordering::Relaxed);
        if let Some(worker) = probe_worker {
            let _ = worker.join();
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: u64 = 1_794_441_000_000;
    const RESET: &str = "2026-11-15T08:00:00Z";

    fn cache(provider: &str, labels: [&str; 2]) -> serde_json::Value {
        json!({"entries": {
            format!("{provider}:fixture"): {
                "fetchedAt": NOW,
                "usage": {"windows": [
                    {"label": labels[0], "usedFraction": 0.0, "resetsAt": RESET},
                    {"label": labels[1], "usedFraction": 0.79, "resetsAt": RESET}
                ]},
                "credentials": "ignored"
            }
        }})
    }

    fn parse(value: &serde_json::Value, now: u64) -> UsageSnapshot {
        parse_cache(&serde_json::to_vec(value).unwrap(), now).unwrap()
    }

    #[test]
    fn uses_fraction_not_percent_and_preserves_zero() {
        let reading = parse(&cache("devin", ["Day", "Week"]), NOW);
        let rows = &reading.providers[0].windows;
        assert_eq!(rows[0].percent_text(), "0%");
        assert_eq!(rows[0].remaining_percent(), Some(100));
        assert_eq!(rows[1].used_fraction, Some(0.79));
        assert_eq!(rows[1].percent_text(), "79%");
        assert_eq!(rows[1].remaining_percent(), Some(21));
        assert!(rows.iter().all(|r| r.state == WindowState::Live));
        assert_eq!(reading.providers[0].problem, None);
        assert_eq!(reading.providers[1].windows[0].percent_text(), "--");
    }

    #[test]
    fn codex_windows_are_mapped_by_label_not_position() {
        let reading = parse(&cache("codex", ["Week", "Session"]), NOW);
        let rows = &reading.providers[1].windows;
        assert_eq!(rows[0].label, "5h");
        assert_eq!(rows[0].percent_text(), "79%");
        assert_eq!(rows[1].label, "7d");
        assert_eq!(rows[1].percent_text(), "0%");
        assert!(reading.providers[0].windows[0].used_fraction.is_none());
    }

    #[test]
    fn invalid_percentages_are_not_clamped_or_made_zero() {
        let mut source = cache("devin", ["Day", "Week"]);
        source["entries"]["devin:fixture"]["usage"]["windows"][0]["usedFraction"] = json!(-0.1);
        source["entries"]["devin:fixture"]["usage"]["windows"][1]["usedFraction"] = json!(79.0);
        let reading = parse(&source, NOW);
        assert!(reading.providers[0]
            .windows
            .iter()
            .all(|r| r.used_fraction.is_none()));
    }

    #[test]
    fn all_invalid_windows_report_a_problem() {
        let mut source = cache("devin", ["Day", "Week"]);
        source["entries"]["devin:fixture"]["usage"]["windows"][0]["usedFraction"] = json!(-0.1);
        source["entries"]["devin:fixture"]["usage"]["windows"][1]["usedFraction"] = json!(79.0);
        let reading = parse(&source, NOW);
        assert_eq!(reading.providers[0].problem, Some("missing-windows"));
    }

    #[test]
    fn stale_age_and_provider_errors_preserve_last_value_with_marker() {
        let mut source = cache("devin", ["Day", "Week"]);
        let reading = parse(&source, NOW + FRESH_FOR_MS + 1);
        assert_eq!(reading.providers[0].windows[1].percent_text(), "~79%");
        source["entries"]["devin:fixture"]["error"] = json!({"kind":"network","detail":"ignored"});
        let reading = parse(&source, NOW);
        assert_eq!(reading.providers[0].problem, Some("network"));
        assert_eq!(reading.providers[0].windows[1].percent_text(), "~79%");
    }

    #[test]
    fn expired_reset_never_fabricates_zero_or_keeps_old_usage() {
        let mut source = cache("devin", ["Day", "Week"]);
        source["entries"]["devin:fixture"]["usage"]["windows"][0]["resetsAt"] =
            json!("2026-11-10T08:00:00Z");
        let reading = parse(&source, NOW);
        assert_eq!(
            reading.providers[0].windows[0].state,
            WindowState::ResetPending
        );
        assert_eq!(reading.providers[0].windows[0].percent_text(), "--");
        assert_eq!(reading.providers[0].windows[1].percent_text(), "79%");
    }

    #[test]
    fn absent_auth_and_timestamps_are_not_usage() {
        let source = json!({"entries":{"codex:fixture":{"error":{"kind":"no-credentials"}}}});
        let reading = parse(&source, NOW);
        assert_eq!(reading.providers[1].problem, Some("no-credentials"));
        assert_eq!(reading.providers[1].windows[0].percent_text(), "--");
        for timestamp in [None, Some(NOW + FUTURE_TOLERANCE_MS + 1)] {
            let mut source = cache("devin", ["Day", "Week"]);
            source["entries"]["devin:fixture"]["fetchedAt"] = json!(timestamp);
            let reading = parse(&source, NOW);
            assert_eq!(reading.providers[0].problem, Some("missing-timestamp"));
            assert!(reading.providers[0]
                .windows
                .iter()
                .all(|r| r.used_fraction.is_none()));
        }
    }

    #[test]
    fn ambiguous_accounts_and_duplicate_windows_are_not_combined() {
        let mut source = cache("devin", ["Day", "Week"]);
        source["entries"]["devin:other"] = source["entries"]["devin:fixture"].clone();
        let reading = parse(&source, NOW);
        assert_eq!(reading.providers[0].problem, Some("multiple-accounts"));
        assert!(reading.providers[0]
            .windows
            .iter()
            .all(|r| r.used_fraction.is_none()));
        let reading = parse(&cache("devin", ["Day", "Day"]), NOW);
        assert!(reading.providers[0]
            .windows
            .iter()
            .all(|r| r.used_fraction.is_none()));
    }

    #[test]
    fn extra_metrics_and_timezone_less_dates_are_not_guessed() {
        let mut source = cache("codex", ["Code review", "Week"]);
        source["entries"]["codex:fixture"]["usage"]["windows"][1]["resetsAt"] =
            json!("2026-11-15T08:00:00");
        let reading = parse(&source, NOW);
        assert!(reading.providers[1]
            .windows
            .iter()
            .all(|r| r.used_fraction.is_none()));
        assert!(parse_cache(b"{", NOW).is_err());
        assert!(parse_cache(b"{}", NOW).is_err());
    }

    #[test]
    fn timezone_offsets_match_same_instant_and_rounding_is_bounded() {
        assert_eq!(
            reset_millis("2026-11-15T18:00:00+10:00"),
            reset_millis(RESET)
        );
        let mut source = cache("devin", ["Day", "Week"]);
        source["entries"]["devin:fixture"]["usage"]["windows"][1]["usedFraction"] = json!(1.0);
        let reading = parse(&source, NOW);
        assert_eq!(reading.providers[0].windows[1].percent_text(), "100%");
        assert_eq!(reading.providers[0].windows[1].remaining_percent(), Some(0));
    }

    #[test]
    fn reader_keeps_last_measurement_stale_on_partial_write_and_stops() {
        use std::io::Write;
        let path = std::env::temp_dir().join(format!(
            "iskra-usage-{}-{}.json",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut source = cache("devin", ["Day", "Week"]);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        source["entries"]["devin:fixture"]["fetchedAt"] = json!(now);
        for row in source["entries"]["devin:fixture"]["usage"]["windows"]
            .as_array_mut()
            .unwrap()
        {
            row["resetsAt"] = json!("2099-11-15T08:00:00Z");
        }
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .unwrap();
        file.write_all(&serde_json::to_vec(&source).unwrap())
            .unwrap();
        drop(file);
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::sync_channel(1);
        // Probe worker disabled — cache-only behavior under test.
        let worker = spawn_cache_reader_impl(path.clone(), tx, stop.clone(), |_, _, _| None);
        let first = rx.recv_timeout(Duration::from_secs(3)).unwrap();
        std::fs::write(&path, b"{").unwrap();
        let second = rx.recv_timeout(Duration::from_secs(3)).unwrap();
        stop.store(true, Ordering::Relaxed);
        worker.join().unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(first.providers[0].windows[1].percent_text(), "79%");
        assert_eq!(second.providers[0].windows[1].percent_text(), "~79%");
        assert_eq!(second.providers[0].problem, Some("cache-unavailable"));
    }

    /// Cache where Codex's entry is unhealthy (no-credentials): the
    /// injected probe result must replace the WHOLE Codex provider as
    /// CodexDirect — never spliced per-window.
    #[test]
    fn reader_merges_direct_codex_when_cache_unhealthy() {
        use std::io::Write;
        let path = std::env::temp_dir().join(format!(
            "iskra-fallback-{}-{}.json",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .unwrap();
        file.write_all(&serde_json::to_vec(&cache("devin", ["Day", "Week"])).unwrap())
            .unwrap();
        drop(file);
        // The synthetic fixture's reset timestamps expire same-day;
        // pin only the resetsAt fields to a fixed future date so the
        // merge stays valid regardless of when the test runs. Percents
        // and durations stay exactly as in the fixture.
        let mut frozen: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/codex-rate-limits.json")).unwrap();
        for window in ["primary", "secondary"] {
            frozen["result"]["rateLimitsByLimitId"]["codex"][window]["resetsAt"] =
                json!(4_102_444_800_i64);
        }
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::sync_channel(1);
        let worker =
            spawn_cache_reader_impl(path.clone(), tx, stop.clone(), move |ptx, needed, _| {
                Some(std::thread::spawn(move || {
                    while !needed.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as u64;
                    let _ = ptx.try_send(Ok(crate::codex_probe::ProbeSample {
                        result: frozen["result"].clone(),
                        fetched_at_ms: now,
                    }));
                }))
            });
        let mut merged = None;
        for _ in 0..25 {
            let s = rx.recv_timeout(Duration::from_secs(4)).unwrap();
            if s.providers[1].source == UsageSource::CodexDirect {
                merged = Some(s);
                break;
            }
        }
        stop.store(true, Ordering::Relaxed);
        worker.join().unwrap();
        std::fs::remove_file(path).unwrap();
        let s = merged.expect("direct Codex sample must win over unhealthy cache");
        let codex = &s.providers[1];
        assert_eq!(codex.windows[0].percent_text(), "37%");
        assert_eq!(codex.windows[1].percent_text(), "55%");
        assert_eq!(codex.problem, None);
        // Devin still comes straight from the Zeron cache.
        assert_eq!(s.providers[0].source, UsageSource::ZeronCache);
    }

    /// A fully live Zeron Codex entry suppresses the fallback: even a
    /// delivered direct sample must not displace it.
    #[test]
    fn reader_keeps_healthy_cache_over_direct() {
        use std::io::Write;
        let path = std::env::temp_dir().join(format!(
            "iskra-healthy-{}-{}.json",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut source = cache("devin", ["Day", "Week"]);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let mut codex = cache("codex", ["5h", "7d"]);
        codex["entries"]["codex:fixture"]["fetchedAt"] = json!(now);
        source["entries"]
            .as_object_mut()
            .unwrap()
            .extend(codex["entries"].as_object().unwrap().clone());
        source["entries"]["devin:fixture"]["fetchedAt"] = json!(now);
        // Synthetic future resets so the live-state check is
        // date-independent — not a real provider report.
        for key in ["devin:fixture", "codex:fixture"] {
            for window in source["entries"][key]["usage"]["windows"]
                .as_array_mut()
                .unwrap()
            {
                window["resetsAt"] = json!("2100-01-01T00:00:00Z");
            }
        }
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .unwrap();
        file.write_all(&serde_json::to_vec(&source).unwrap())
            .unwrap();
        drop(file);
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::sync_channel(1);
        // Fake probe that would deliver a sample if ever asked — it
        // must never be asked (needed stays false).
        let asked = Arc::new(AtomicBool::new(false));
        let asked_c = asked.clone();
        let worker =
            spawn_cache_reader_impl(path.clone(), tx, stop.clone(), move |_, needed, _| {
                Some(std::thread::spawn(move || {
                    for _ in 0..60 {
                        if needed.load(Ordering::Relaxed) {
                            asked_c.store(true, Ordering::Relaxed);
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                }))
            });
        let _ = rx.recv_timeout(Duration::from_secs(3)).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        stop.store(true, Ordering::Relaxed);
        worker.join().unwrap();
        std::fs::remove_file(path).unwrap();
        assert!(
            !asked.load(Ordering::Relaxed),
            "healthy cache → needed=false"
        );
    }

    #[test]
    fn oversized_or_missing_cache_is_rejected() {
        use std::io::Write;
        let path = std::env::temp_dir().join(format!(
            "iskra-oversized-{}-{}.json",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        assert!(read_cache(&path).is_none());
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .unwrap();
        file.write_all(&vec![b' '; MAX_CACHE_BYTES as usize + 1])
            .unwrap();
        drop(file);
        assert!(read_cache(&path).is_none());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    #[ignore = "lead-only check against a sanitized frozen source capture"]
    fn captured_cache_diagnostic() {
        let bytes = std::fs::read(std::env::var_os("ISKRA_USAGE_CAPTURE").unwrap()).unwrap();
        let raw: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let now = raw["capturedAtMs"].as_u64().unwrap();
        let reading = parse_cache(&bytes, now).unwrap();
        for provider in reading.providers {
            println!(
                "{:?}: problem={:?}, fetched={:?}",
                provider.provider, provider.problem, provider.fetched_at_ms
            );
            for window in provider.windows {
                println!(
                    "  {} used={} remaining={:?} state={:?} reset={:?}",
                    window.label,
                    window.percent_text(),
                    window.remaining_percent(),
                    window.state,
                    window.resets_at
                );
            }
        }
    }
}
