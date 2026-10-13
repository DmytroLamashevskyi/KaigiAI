//! On-device inference servers (whisper.cpp / llama.cpp) run as child processes.
//!
//! Local mode does NOT link whisper/llama in-process — both `-sys` crates vendor
//! their own `ggml` and the duplicate symbols fail to link (docs/PROJECT.md §10.3).
//! Instead the user installs the servers, points settings at the executables, and
//! we spawn them here, talking to them over their OpenAI-compatible HTTP API via
//! [`crate::provider::api::ApiProvider`].

use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub const DEFAULT_WHISPER_PORT: u16 = 8771;
pub const DEFAULT_LLAMA_PORT: u16 = 8770;

/// How long to wait for a freshly-spawned server to become ready: a base 45 s
/// plus 15 s per GB of model file, capped at 5 min. Recent llama-server builds
/// bind the port only AFTER the model is loaded, and loading is disk-bound —
/// a 9 GB Qwen2.5-14B took 76 s cold, past the old fixed 45 s, so big models
/// could never start (§10.19). Still finite, so a broken setup shows an error
/// instead of an endless "preparing". The frontend's safety cap is above the max.
fn startup_timeout(model_path: &str) -> Duration {
    let gb = std::fs::metadata(model_path)
        .map(|m| m.len() as f64 / 1e9)
        .unwrap_or(0.0);
    Duration::from_secs_f64((45.0 + 15.0 * gb).min(300.0))
}

/// llama-server context (tokens, shared by all slots). Without `-c` it takes
/// the model's full training context — 32k for Qwen2.5, ~6 GB of KV cache on
/// a 14B model, which left whisper large-v3 + Qwen 14B at 15.9 / 16 GB VRAM.
/// The largest request we send is a meeting summary: ≤24k chars of transcript
/// (≈9k tokens) + a 2048-token reply, so 16k is ample.
const LLAMA_CTX: &str = "16384";

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
        let timeout = startup_timeout(&model);
        let args = vec![
            "-m".into(), model, "--host".into(), "127.0.0.1".into(),
        ];
        let port = self.ensure(Which::Whisper, sig, exe, args, None, DEFAULT_WHISPER_PORT, timeout)?;
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
        let timeout = startup_timeout(&model);
        let mut args = vec![
            "-m".into(), model, "--host".into(), "127.0.0.1".into(), "-ngl".into(), ngl.to_string(),
            "-c".into(), LLAMA_CTX.into(),
        ];
        // The host-RAM prompt cache (on by default in recent builds) costs a
        // median ~0.6 s and up to 15 s per request saving prompts we never
        // reuse — every translation is a fresh short prompt. Disabling it cut 6
        // parallel translations from 7.0 s to 2.1 s median (FLEURS benchmark,
        // §10.19). Older builds reject unknown flags, so probe `--help` first.
        if supports_flag(&exe, "--cache-ram") {
            args.extend(["--cache-ram".into(), "0".into()]);
        }
        // Older llama.cpp builds bind before the model loads — gate on /health
        // (200 = ready) either way.
        let port =
            self.ensure(Which::Llama, sig, exe, args, Some("/health"), DEFAULT_LLAMA_PORT, timeout)?;
        Ok(base_url(port))
    }

    /// Spawn (or reuse) a server and wait until it is ready — WITHOUT holding the
    /// state lock across the multi-second wait. The lock is taken only for brief
    /// reuse/spawn/flip-ready steps; the TCP/health probes and the 300ms sleeps
    /// run unlocked, so a slow start can't freeze other server operations. A
    /// concurrent caller for the same (not-yet-ready) server waits on it rather
    /// than spawning a duplicate. `--port` is appended here.
    #[allow(clippy::too_many_arguments)]
    fn ensure(
        &self,
        which: Which,
        sig: String,
        exe: String,
        mut args: Vec<String>,
        health: Option<&'static str>,
        preferred: u16,
        timeout: Duration,
    ) -> Result<u16, String> {
        let deadline = Instant::now() + timeout;
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

/// Whether a server executable lists `flag` in its `--help`. Cached per exe
/// (one ~0.7 s probe per path per app run); `false` if the probe fails.
fn supports_flag(exe: &str, flag: &str) -> bool {
    static CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let mut map = cache.lock().unwrap_or_else(|e| e.into_inner());
    let help = map.entry(exe.to_string()).or_insert_with(|| {
        let mut cmd = Command::new(exe);
        cmd.arg("--help").stdin(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        cmd.output()
            .map(|o| {
                let mut text = String::from_utf8_lossy(&o.stdout).into_owned();
                text.push_str(&String::from_utf8_lossy(&o.stderr));
                text
            })
            .unwrap_or_default()
    });
    help.contains(flag)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_timeout_scales_with_model_size() {
        // Missing file: the base timeout.
        assert_eq!(startup_timeout("no/such/model.gguf"), Duration::from_secs(45));
        let path = std::env::temp_dir().join(format!("kaigi_model_{}.bin", std::process::id()));
        let f = std::fs::File::create(&path).unwrap();
        f.set_len(2_000_000_000).unwrap(); // sparse 2 GB
        drop(f);
        assert_eq!(startup_timeout(path.to_str().unwrap()), Duration::from_secs(75));
        let _ = std::fs::remove_file(&path);
    }
}
