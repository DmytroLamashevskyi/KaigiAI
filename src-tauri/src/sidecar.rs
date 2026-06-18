//! On-device inference servers (whisper.cpp / llama.cpp) run as child processes.
//!
//! Local mode does NOT link whisper/llama in-process — both `-sys` crates vendor
//! their own `ggml` and the duplicate symbols fail to link (docs/PROJECT.md §10.3).
//! Instead the user installs the servers, points settings at the executables, and
//! we spawn them here, talking to them over their OpenAI-compatible HTTP API via
//! [`crate::provider::api::ApiProvider`].

use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const DEFAULT_WHISPER_PORT: u16 = 8771;
pub const DEFAULT_LLAMA_PORT: u16 = 8770;

/// How long to wait for a freshly-spawned server to start listening. Model load
/// (esp. whisper large / big GGUF) can take a while, but if it hasn't bound the
/// port in this window something is wrong — fail fast so the UI shows an error
/// instead of an endless "preparing".
const STARTUP_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Default)]
pub struct Sidecars {
    inner: Mutex<Servers>,
}

#[derive(Default)]
struct Servers {
    whisper: Option<Running>,
    llama: Option<Running>,
}

struct Running {
    child: Child,
    /// Config signature: a change here means we must restart the server.
    sig: String,
    port: u16,
    /// Whether the server has been confirmed listening/healthy. While `false`
    /// the server is still starting (one caller spawned it; concurrent callers
    /// wait on it rather than spawning a duplicate).
    ready: bool,
}

impl Running {
    fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Clone, Copy)]
enum Which {
    Whisper,
    Llama,
}

impl Servers {
    fn slot(&mut self, which: Which) -> &mut Option<Running> {
        match which {
            Which::Whisper => &mut self.whisper,
            Which::Llama => &mut self.llama,
        }
    }
}

fn s(settings: &serde_json::Value, key: &str) -> String {
    settings.get(key).and_then(|v| v.as_str()).unwrap_or("").trim().to_string()
}

impl Sidecars {
    /// Lock the inner state, recovering from a poisoned lock instead of panicking
    /// (a panic while another caller held it must not brick all server management).
    fn lock(&self) -> std::sync::MutexGuard<'_, Servers> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Ensure the whisper server is running for the current settings; returns its
    /// base URL at the server root (whisper.cpp serves `/inference` there, no `/v1`).
    pub fn ensure_whisper(&self, settings: &serde_json::Value) -> Result<String, String> {
        let exe = s(settings, "localWhisperServerPath");
        let model = s(settings, "localWhisperPath");
        if exe.is_empty() {
            return Err("Set the whisper-server path in Settings (local mode).".into());
        }
        if model.is_empty() {
            return Err("Set the Whisper model path in Settings (local mode).".into());
        }
        let sig = format!("{exe}|{model}");
        let args = vec![
            "-m".into(), model, "--host".into(), "127.0.0.1".into(),
        ];
        let port = self.ensure(Which::Whisper, sig, exe, args, None, DEFAULT_WHISPER_PORT)?;
        Ok(base_url_root(port))
    }

    /// Ensure the llama server is running for the current settings; returns its
    /// OpenAI-compatible base URL.
    pub fn ensure_llama(&self, settings: &serde_json::Value) -> Result<String, String> {
        let exe = s(settings, "localLlmServerPath");
        let model = s(settings, "localLlmPath");
        let ngl = settings.get("nGpuLayers").and_then(|v| v.as_i64()).unwrap_or(0);
        if exe.is_empty() {
            return Err("Set the llama-server path in Settings (local mode).".into());
        }
        if model.is_empty() {
            return Err("Set the LLM model path in Settings (local mode).".into());
        }
        let sig = format!("{exe}|{model}|{ngl}");
        let args = vec![
            "-m".into(), model, "--host".into(), "127.0.0.1".into(), "-ngl".into(), ngl.to_string(),
        ];
        // llama.cpp binds before the model loads — gate on /health (200 = ready).
        let port = self.ensure(Which::Llama, sig, exe, args, Some("/health"), DEFAULT_LLAMA_PORT)?;
        Ok(base_url(port))
    }

    /// Spawn (or reuse) a server and wait until it is ready — WITHOUT holding the
    /// state lock across the multi-second wait. The lock is taken only for brief
    /// reuse/spawn/flip-ready steps; the TCP/health probes and the 300ms sleeps
    /// run unlocked, so a slow start can't freeze other server operations. A
    /// concurrent caller for the same (not-yet-ready) server waits on it rather
    /// than spawning a duplicate. `--port` is appended here.
    fn ensure(
        &self,
        which: Which,
        sig: String,
        exe: String,
        mut args: Vec<String>,
        health: Option<&'static str>,
        preferred: u16,
    ) -> Result<u16, String> {
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        loop {
            // Phase A — brief lock: reuse a ready server, or spawn/adopt one.
            let port = {
                let mut g = self.lock();
                let slot = g.slot(which);
                match slot {
                    Some(r) if r.sig == sig => {
                        if r.ready {
                            if r.alive() {
                                return Ok(r.port);
                            }
                            r.kill();
                            *slot = None; // ready but died → respawn below
                        }
                        // else: still starting (us or a concurrent caller) → wait on it
                    }
                    Some(r) => {
                        // Different config → restart with the new settings.
                        r.kill();
                        *slot = None;
                    }
                    None => {}
                }
                if slot.is_none() {
                    let p = pick_port(preferred);
                    args.push("--port".into());
                    args.push(p.to_string());
                    let arg_refs: Vec<&str> = args.iter().map(|x| x.as_str()).collect();
                    let child = spawn(&exe, &arg_refs)?;
                    args.truncate(args.len() - 2); // drop port for a possible retry
                    *slot = Some(Running { child, sig: sig.clone(), port: p, ready: false });
                }
                slot.as_ref().unwrap().port
            };

            // Phase B — unlocked probe.
            let addr: std::net::SocketAddr = format!("127.0.0.1:{port}")
                .parse()
                .map_err(|e| format!("bad addr: {e}"))?;
            let listening = TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok();
            let ready = listening
                && match health {
                    None => true,
                    Some(p) => http_status(&addr, p) == Some(200),
                };
            if ready {
                let mut g = self.lock();
                if let Some(r) = g.slot(which) {
                    if r.port == port {
                        r.ready = true;
                        return Ok(port);
                    }
                }
                continue; // replaced under us — re-evaluate
            }

            // Detect a server that exited during startup (brief lock).
            {
                let mut g = self.lock();
                let slot = g.slot(which);
                match slot {
                    Some(r) if r.port == port => {
                        if matches!(r.child.try_wait(), Ok(Some(_))) {
                            *slot = None;
                            return Err("server exited during startup".into());
                        }
                    }
                    _ => continue, // replaced under us
                }
            }

            if Instant::now() >= deadline {
                let mut g = self.lock();
                let slot = g.slot(which);
                if let Some(r) = slot {
                    if r.port == port {
                        r.kill();
                        *slot = None;
                    }
                }
                return Err("server did not start listening in time".into());
            }
            std::thread::sleep(Duration::from_millis(300));
        }
    }

    /// Kill any running servers. Called on app exit.
    pub fn shutdown(&self) {
        let mut guard = self.lock();
        if let Some(mut r) = guard.whisper.take() {
            r.kill();
        }
        if let Some(mut r) = guard.llama.take() {
            r.kill();
        }
    }

    /// Kill a specific server unconditionally (used to clean up after a failed
    /// recording start so a just-spawned server isn't left orphaned).
    fn kill(&self, which: Which) {
        let mut g = self.lock();
        if let Some(mut r) = g.slot(which).take() {
            r.kill();
        }
    }

    /// Kill the whisper server (public wrapper for the recording-start error path).
    pub fn kill_whisper(&self) {
        self.kill(Which::Whisper);
    }
}

impl Drop for Sidecars {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn base_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/v1")
}

/// Server-root base URL (no `/v1`). whisper.cpp's transcription route lives here.
fn base_url_root(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

fn spawn(exe: &str, args: &[&str]) -> Result<Child, String> {
    let mut cmd = Command::new(exe);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn().map_err(|e| format!("failed to start '{exe}': {e}"))
}

/// Minimal blocking HTTP/1.0 GET; returns the response status code, if any.
/// Used only for the local readiness probe, so no body parsing is needed.
fn http_status(addr: &std::net::SocketAddr, path: &str) -> Option<u16> {
    use std::io::{Read, Write};
    let mut stream = TcpStream::connect_timeout(addr, Duration::from_millis(500)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    stream.set_write_timeout(Some(Duration::from_secs(2))).ok()?;
    let req = format!("GET {path} HTTP/1.0\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).ok()?;
    let mut buf = [0u8; 256];
    let n = stream.read(&mut buf).ok()?;
    // Status line looks like: "HTTP/1.1 200 OK"
    let head = String::from_utf8_lossy(&buf[..n]);
    head.lines().next()?.split_whitespace().nth(1)?.parse().ok()
}

/// Use the preferred port if free, otherwise grab an ephemeral one.
fn pick_port(preferred: u16) -> u16 {
    if TcpListener::bind(("127.0.0.1", preferred)).is_ok() {
        return preferred;
    }
    TcpListener::bind(("127.0.0.1", 0))
        .ok()
        .and_then(|l| l.local_addr().ok())
        .map(|a| a.port())
        .unwrap_or(preferred)
}
