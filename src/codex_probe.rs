//! Direct Codex app-server quota probe: spawn `codex app-server`, send the
//! lead-authored initialize + account/rateLimits/read requests over
//! newline-delimited JSON-RPC, return the result object. No credentials
//! are read, no model turns are started, and nothing is logged from the
//! wire. Every probe is an ephemeral child that is always reaped.
use crate::codex_limits::{self, ProbeError};
use serde_json::Value;
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::io::AsRawFd;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug)]
pub struct ProbeSample {
    /// The `result` object of the rateLimits reply — never the envelope.
    pub result: Value,
    /// Unix ms captured at successful reply completion.
    pub fetched_at_ms: u64,
}

/// Program for production probes: the native CLI when present, otherwise
/// whatever `codex` resolves on PATH (no shell is ever invoked).
/// `ISKRA_CODEX_PROBE` overrides the path when a nonstandard CLI
/// install must be probed; tests use explicit paths instead.
fn default_program() -> PathBuf {
    if let Some(p) = std::env::var_os("ISKRA_CODEX_PROBE") {
        return PathBuf::from(p);
    }
    const NATIVE: &str = "/usr/bin/codex";
    if Path::new(NATIVE).exists() {
        PathBuf::from(NATIVE)
    } else {
        PathBuf::from("codex")
    }
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Feed a chunk of child stdout into `buf` under the shared 1MiB
/// cumulative bound and drain complete newline-delimited replies out of
/// it, returning the response for `id`. Notifications and other ids are
/// skipped inside the same bound; partial lines stay buffered.
fn ingest(
    bytes: &[u8],
    buf: &mut Vec<u8>,
    total: &mut usize,
    id: u64,
) -> Result<Option<Value>, ProbeError> {
    *total += bytes.len();
    if *total > codex_limits::MAX_RESPONSE_BYTES {
        return Err(ProbeError::Protocol); // oversize
    }
    buf.extend_from_slice(bytes);
    while let Some(nl) = buf.iter().position(|b| *b == b'\n') {
        let line: Vec<u8> = buf.drain(..=nl).collect();
        let text = match std::str::from_utf8(&line) {
            Ok(t) => t.trim(),
            Err(_) => return Err(ProbeError::Protocol),
        };
        if text.is_empty() {
            continue;
        }
        let msg: Value = serde_json::from_str(text).map_err(|_| ProbeError::Protocol)?;
        let Some(msg_id) = msg.get("id") else {
            continue; // notification: skipped under the same bound
        };
        if msg_id.as_u64() != Some(id) {
            continue;
        }
        if let Some(error) = msg.get("error").filter(|e| e.is_object()) {
            return Err(codex_limits::rpc_error(error));
        }
        if let Some(result) = msg.get("result") {
            return Ok(Some(result.clone()));
        }
        return Err(ProbeError::Protocol);
    }
    Ok(None)
}

/// Read one JSON-RPC response carrying `id` from the child's stdout.
/// Unrelated notifications and other ids are skipped inside the shared
/// 1MiB cumulative byte bound and the session deadline. Polls in 100ms
/// slices so `stop` cancels promptly without a reader thread.
#[cfg(unix)]
fn await_response(
    child: &mut Child,
    buf: &mut Vec<u8>,
    total: &mut usize,
    id: u64,
    deadline: Instant,
    stop: &AtomicBool,
) -> Result<Value, ProbeError> {
    let stdout = child.stdout.as_mut().ok_or(ProbeError::Unavailable)?;
    let fd = stdout.as_raw_fd();
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags == -1 || libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) == -1 {
            // Without O_NONBLOCK a silent child would block the reader
            // past the deadline — treat as unavailable, not a hang.
            return Err(ProbeError::Unavailable);
        }
    }
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let mut chunk = [0u8; 8192];
    loop {
        if stop.load(Ordering::Relaxed) {
            return Err(ProbeError::Unavailable);
        }
        if Instant::now() >= deadline {
            return Err(ProbeError::Timeout);
        }
        // saturating: a second Instant::now() may already be past deadline.
        let timeout = deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(100))
            .as_millis() as libc::c_int;
        let ready = unsafe { libc::poll(&mut pfd, 1, timeout.max(0)) };
        if ready < 0 {
            if unsafe { *libc::__errno_location() } == libc::EINTR {
                continue;
            }
            return Err(ProbeError::Unavailable);
        }
        // A lone POLLERR/POLLNVAL is terminal — re-polling it would spin
        // at 100% CPU until the deadline.
        if ready > 0 && pfd.revents & (libc::POLLERR | libc::POLLNVAL) != 0 {
            return Err(ProbeError::Protocol);
        }
        if ready > 0 && pfd.revents & (libc::POLLIN | libc::POLLHUP) != 0 {
            match stdout.read(&mut chunk) {
                Ok(0) => return Err(ProbeError::Protocol), // peer closed
                Ok(n) => {
                    if let Some(v) = ingest(&chunk[..n], buf, total, id)? {
                        return Ok(v);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => return Err(ProbeError::Protocol),
            }
        }
    }
}

/// Windows variant: `PeekNamedPipe` is the readiness check a pollfd
/// provides on unix — a blocking pipe read returns as soon as any bytes
/// are present, so a nonzero peek means the following read can't stall.
/// Slices stay at 100ms so `stop` and the deadline behave identically.
#[cfg(windows)]
fn await_response(
    child: &mut Child,
    buf: &mut Vec<u8>,
    total: &mut usize,
    id: u64,
    deadline: Instant,
    stop: &AtomicBool,
) -> Result<Value, ProbeError> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Pipes::PeekNamedPipe;
    let stdout = child.stdout.as_mut().ok_or(ProbeError::Unavailable)?;
    let handle = stdout.as_raw_handle() as windows_sys::Win32::Foundation::HANDLE;
    let mut chunk = [0u8; 8192];
    loop {
        if stop.load(Ordering::Relaxed) {
            return Err(ProbeError::Unavailable);
        }
        if Instant::now() >= deadline {
            return Err(ProbeError::Timeout);
        }
        let mut avail = 0u32;
        if unsafe {
            PeekNamedPipe(
                handle,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &mut avail,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(ProbeError::Protocol); // write end closed
        }
        if avail == 0 {
            std::thread::sleep(
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(100)),
            );
            continue;
        }
        match stdout.read(&mut chunk) {
            Ok(0) => return Err(ProbeError::Protocol), // peer closed
            Ok(n) => {
                if let Some(v) = ingest(&chunk[..n], buf, total, id)? {
                    return Ok(v);
                }
            }
            Err(_) => return Err(ProbeError::Protocol),
        }
    }
}

fn send(child: &mut Child, msg: &Value) -> Result<(), ProbeError> {
    let stdin = child.stdin.as_mut().ok_or(ProbeError::Unavailable)?;
    stdin
        .write_all(msg.to_string().as_bytes())
        .and_then(|()| stdin.write_all(b"\n"))
        .and_then(|()| stdin.flush())
        .map_err(|_| ProbeError::Network)
}

/// One ephemeral probe against `program`'s app-server. Sends exactly the
/// lead-authored initialize + initialized + limits requests, awaits the
/// replies by id, and always kills/reaps the child — including on
/// timeout, stop, and protocol failure.
pub fn probe(program: &Path, stop: &AtomicBool) -> Result<ProbeSample, ProbeError> {
    let deadline = Instant::now() + codex_limits::REQUEST_TIMEOUT;
    // Tether the child to this process: if the app exits while a probe
    // is in flight, the app-server dies with it instead of leaking.
    // unix does this with PR_SET_PDEATHSIG + a parent-pid re-check in
    // pre_exec; Windows assigns the child to a KILL_ON_JOB_CLOSE job.
    #[cfg(unix)]
    let parent_pid = std::process::id();
    let mut cmd = Command::new(program);
    cmd.arg("app-server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    unsafe {
        cmd.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::getppid() != parent_pid as libc::pid_t {
                // Parent already exited between spawn and exec — abort
                // rather than start an orphan app-server.
                return Err(std::io::Error::from_raw_os_error(libc::EINTR));
            }
            Ok(())
        });
    }
    let spawn = cmd.spawn();
    let mut child = match spawn {
        Ok(c) => c,
        Err(_) => return Err(ProbeError::Unavailable),
    };
    // Without the tether a crashed app could orphan a live app-server;
    // a job that can't be created makes the probe unsafe to run.
    #[cfg(windows)]
    let _tether = match JobTether::tether(&child) {
        Some(t) => t,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ProbeError::Unavailable);
        }
    };
    let mut buf = Vec::new();
    let mut total = 0usize;
    let result = (|| {
        send(&mut child, &codex_limits::initialize_request())?;
        await_response(&mut child, &mut buf, &mut total, 1, deadline, stop)?;
        send(&mut child, &codex_limits::initialized_notification())?;
        send(&mut child, &codex_limits::limits_request())?;
        let result = await_response(&mut child, &mut buf, &mut total, 2, deadline, stop)?;
        Ok(ProbeSample {
            result,
            fetched_at_ms: unix_ms(),
        })
    })();
    // Always reap this child — timeout/stop/failure included.
    let _ = child.kill();
    let _ = child.wait();
    result
}

/// A Job Object holding the probe child with JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE:
/// the PR_SET_PDEATHSIG equivalent. The handle must outlive the probe —
/// closing the last job handle terminates every assigned process, which is
/// also what guarantees the kill if this process dies mid-probe.
#[cfg(windows)]
struct JobTether(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl JobTether {
    fn tether(child: &Child) -> Option<JobTether> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::JobObjects::*;
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return None;
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) == 0
            {
                CloseHandle(job);
                return None;
            }
            if AssignProcessToJobObject(job, child.as_raw_handle() as _) == 0 {
                CloseHandle(job);
                return None;
            }
            Some(JobTether(job))
        }
    }
}

#[cfg(windows)]
impl Drop for JobTether {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

/// Background worker: probes only while `needed`, immediately on the
/// first needed tick and at most once per REFRESH_INTERVAL after that.
/// Sleeps in 100ms slices so `stop` is always responsive. Sends are
/// non-blocking retries so a full channel can't wedge the worker.
pub fn spawn_reader(
    tx: mpsc::SyncSender<Result<ProbeSample, ProbeError>>,
    needed: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
) -> JoinHandle<()> {
    spawn_reader_impl(tx, needed, stop, default_program())
}

/// Same worker with an explicit program path — lets tests pass a fake
/// executable without touching the shared ISKRA_CODEX_PROBE env var
/// (parallel test workers would race on it).
fn spawn_reader_impl(
    tx: mpsc::SyncSender<Result<ProbeSample, ProbeError>>,
    needed: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    program: PathBuf,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut last_probe: Option<Instant> = None;
        while !stop.load(Ordering::Relaxed) {
            if needed.load(Ordering::Relaxed)
                && last_probe.is_none_or(|t| t.elapsed() >= codex_limits::REFRESH_INTERVAL)
            {
                last_probe = Some(Instant::now());
                let mut out = probe(&program, &stop);
                loop {
                    match tx.try_send(out) {
                        Ok(()) => break,
                        // Receiver gone — nothing will ever read results;
                        // exit instead of polling every REFRESH_INTERVAL.
                        Err(mpsc::TrySendError::Disconnected(_)) => return,
                        Err(mpsc::TrySendError::Full(back)) => {
                            out = back;
                            if stop.load(Ordering::Relaxed) {
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(100));
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    })
}

#[cfg(all(test, unix))] // fixtures are #!/bin/sh fake app-servers
mod tests {
    use super::*;
    use serde_json::json;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "iskra-probe-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn fake_cli(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        // Other test threads fork concurrently. Keep the executable's write
        // descriptor out of this process so their children cannot inherit it
        // and cause ETXTBSY after the creating thread has closed its copy.
        let mut writer = Command::new("/bin/sh")
            .args(["-c", "cat > \"$1\" && chmod 755 \"$1\"", "fixture-writer"])
            .arg(&p)
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        let write = writer
            .stdin
            .take()
            .unwrap()
            .write_all(format!("#!/bin/sh\n{body}\n").as_bytes());
        let status = writer.wait().unwrap();
        write.unwrap();
        assert!(status.success(), "fixture writer failed: {status}");
        p
    }

    fn no_stop() -> AtomicBool {
        AtomicBool::new(false)
    }

    #[test]
    fn probe_sends_exact_requests_and_returns_frozen_result() {
        let dir = tmpdir("ok");
        let log = dir.join("requests.log");
        let frozen: Value =
            serde_json::from_str(include_str!("../tests/fixtures/codex-rate-limits.json")).unwrap();
        let reply = json!({"id":2,"result":frozen["result"].clone()}).to_string();
        let cli = fake_cli(
            &dir,
            "codex-fake",
            &format!(
                r#"[ "$1" = "app-server" ] || exit 3
while IFS= read -r line; do
  printf '%s\n' "$line" >> "{log}"
  case "$line" in
    *'"id":1'*) printf '%s\n' '{{"method":"noisy/notification","params":{{}}}}'
               printf '%s\n' '{{"id":1,"result":{{"ok":true}}}}' ;;
    *'"id":2'*) printf '%s\n' '{reply}' ;;
  esac
done"#,
                log = log.display(),
                reply = reply
            ),
        );
        let out = probe(&cli, &no_stop()).unwrap();
        assert_eq!(out.result, frozen["result"]);
        let now = unix_ms();
        assert!(out.fetched_at_ms > 0 && out.fetched_at_ms <= now);
        let sent = std::fs::read_to_string(&log).unwrap();
        let lines: Vec<&str> = sent.lines().collect();
        assert_eq!(lines.len(), 3, "exactly three wire writes: {sent}");
        let init: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(init, codex_limits::initialize_request());
        let notif: Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(notif, codex_limits::initialized_notification());
        let limits: Value = serde_json::from_str(lines[2]).unwrap();
        assert_eq!(limits, codex_limits::limits_request());
    }

    #[test]
    fn probe_rejects_bad_json_eof_oversize_and_wrong_ids_still_works() {
        let dir = tmpdir("bad");
        let cases = [
            ("garbage", "printf 'this is not json\\n'\nexec sleep 60"),
            ("eof", "head -n 1 >/dev/null\nexit 0"),
            (
                "oversize",
                "head -c 1100000 /dev/zero | tr '\\0' 'x'\nexec sleep 60",
            ),
        ];
        for (name, body) in cases {
            let cli = fake_cli(&dir, name, body);
            assert_eq!(
                probe(&cli, &no_stop()).unwrap_err(),
                ProbeError::Protocol,
                "{name}"
            );
        }
        // Wrong ids and notifications are skipped until the real reply.
        let cli = fake_cli(
            &dir,
            "ids",
            r#"while IFS= read -r line; do
  case "$line" in
    *'"id":1'*) printf '%s\n' '{"id":99,"result":{"decoy":true}}'
               printf '%s\n' '{"method":"tick","params":{}}'
               printf '%s\n' '{"id":1,"result":{"ok":true}}' ;;
    *'"id":2'*) printf '%s\n' '{"id":2,"result":{"windows":[]}}' ;;
  esac
done"#,
        );
        let out = probe(&cli, &no_stop()).unwrap();
        assert_eq!(out.result, json!({"windows": []}));
        // A spawn failure is Unavailable, not a panic.
        assert_eq!(
            probe(Path::new("/nonexistent/codex"), &no_stop()).unwrap_err(),
            ProbeError::Unavailable
        );
    }

    #[test]
    fn probe_times_out_stops_promptly_and_reaps_children() {
        let dir = tmpdir("slow");
        // The fake records its own pid so the test can verify the child
        // was killed+reaped after a cancelled or timed-out probe.
        let pidfile = dir.join("pid");
        let cli = fake_cli(
            &dir,
            "slow",
            &format!("echo $$ > \"{}\"\nexec sleep 60", pidfile.display()),
        );
        let stop = AtomicBool::new(false);
        let started = Instant::now();
        let err = probe(&cli, &stop).unwrap_err();
        let elapsed = started.elapsed();
        assert_eq!(err, ProbeError::Timeout);
        assert!(
            elapsed >= codex_limits::REQUEST_TIMEOUT
                && elapsed < codex_limits::REQUEST_TIMEOUT + Duration::from_secs(5),
            "timeout respected the deadline: {elapsed:?}"
        );
        std::thread::sleep(Duration::from_millis(200));
        let pid = std::fs::read_to_string(&pidfile).unwrap();
        let status = std::process::Command::new("kill")
            .args(["-0", pid.trim()])
            .status()
            .unwrap();
        assert!(!status.success(), "timed-out child must be reaped");

        // Stop cancels promptly, well before the 15s deadline.
        let cli2 = fake_cli(&dir, "slow2", "exec sleep 60");
        let stop2 = Arc::new(AtomicBool::new(false));
        let stop_c = stop2.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            stop_c.store(true, Ordering::Relaxed);
        });
        let started = Instant::now();
        let err = probe(&cli2, &stop2).unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(err, ProbeError::Unavailable);
    }

    #[test]
    fn reader_only_probes_while_needed_and_rate_limits() {
        let dir = tmpdir("worker");
        let marker = dir.join("spawned");
        // Each spawn appends a marker and replies a minimal session.
        let cli = fake_cli(
            &dir,
            "codex-mark",
            &format!(
                r#"echo x >> "{marker}"
while IFS= read -r line; do
  case "$line" in
    *'"id":1'*) printf '%s\n' '{{"id":1,"result":{{}}}}' ;;
    *'"id":2'*) printf '%s\n' '{{"id":2,"result":{{}}}}' ;;
  esac
done"#,
                marker = marker.display()
            ),
        );
        let (tx, rx) = mpsc::sync_channel::<Result<ProbeSample, ProbeError>>(1);
        let needed = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let handle = spawn_reader_impl(tx, needed.clone(), stop.clone(), cli);
        // needed=false: nothing spawns.
        std::thread::sleep(Duration::from_millis(400));
        assert!(!marker.exists(), "no probe while needed=false");
        // needed=true: first probe is immediate.
        needed.store(true, Ordering::Relaxed);
        let first = rx.recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
        assert!(first.result.is_object());
        assert!(marker.exists());
        // needed flaps false→true within REFRESH_INTERVAL: no retry.
        needed.store(false, Ordering::Relaxed);
        needed.store(true, Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(700));
        let spawns = std::fs::read_to_string(&marker).unwrap().lines().count();
        assert_eq!(spawns, 1, "one probe per REFRESH_INTERVAL, no flap retry");
        stop.store(true, Ordering::Relaxed);
        handle.join().unwrap();
    }

    /// A dropped receiver must exit the worker promptly — not poll
    /// forever while needed stays true.
    #[test]
    fn reader_exits_when_receiver_is_dropped() {
        let dir = tmpdir("drop");
        let cli = fake_cli(
            &dir,
            "codex-mark",
            r#"while IFS= read -r line; do
  case "$line" in
    *'"id":1'*) printf '%s\n' '{"id":1,"result":{}}' ;;
    *'"id":2'*) printf '%s\n' '{"id":2,"result":{}}' ;;
  esac
done"#,
        );
        let (tx, rx) = mpsc::sync_channel::<Result<ProbeSample, ProbeError>>(1);
        let needed = Arc::new(AtomicBool::new(true));
        let stop = Arc::new(AtomicBool::new(false));
        let handle = spawn_reader_impl(tx, needed, stop, cli);
        drop(rx);
        // Even mid-probe the worker must notice the closed channel and
        // return — well under REFRESH_INTERVAL, no stop flag needed.
        let done = Instant::now();
        handle.join().unwrap();
        assert!(
            done.elapsed() < Duration::from_secs(20),
            "worker outlived its receiver: {:?}",
            done.elapsed()
        );
    }
}
