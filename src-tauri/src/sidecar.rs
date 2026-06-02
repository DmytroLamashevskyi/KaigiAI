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

fn s(settings: &serde_json::Value, key: &str) -> String {
    settings.get(key).and_then(|v| v.as_str()).unwrap_or("").trim().to_string()
}

impl Sidecars {
    /// Ensure the whisper server is running for the current settings; returns its
    /// OpenAI-compatible base URL (`http://127.0.0.1:<port>/v1`).
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
        let mut guard = self.inner.lock().unwrap();
        if let Some(r) = guard.whisper.as_mut() {
            if r.sig == sig && r.alive() {
                // Reuse path must match the spawn path: server root, no `/v1`.
                return Ok(base_url_root(r.port));
            }
            r.kill();
            guard.whisper = None;
        }
        let port = pick_port(DEFAULT_WHISPER_PORT);
        let child = spawn(&exe, &["-m", &model, "--host", "127.0.0.1", "--port", &port.to_string()])?;
        let mut running = Running { child, sig, port };
        wait_ready(&mut running, None)?;
        guard.whisper = Some(running);
        // whisper.cpp serves `/inference` at the server root, not under `/v1`.
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
        let mut guard = self.inner.lock().unwrap();
        if let Some(r) = guard.llama.as_mut() {
            if r.sig == sig && r.alive() {
                return Ok(base_url(r.port));
            }
            r.kill();
            guard.llama = None;
        }
        let port = pick_port(DEFAULT_LLAMA_PORT);
        let child = spawn(
            &exe,
            &[
                "-m", &model,
                "--host", "127.0.0.1",
                "--port", &port.to_string(),
                "-ngl", &ngl.to_string(),
            ],
        )?;
        let mut running = Running { child, sig, port };
        // llama.cpp binds before the model loads — gate on /health (200 = ready).
        wait_ready(&mut running, Some("/health"))?;
        guard.llama = Some(running);
        Ok(base_url(port))
    }

    /// Kill any running servers. Called on app exit.
    pub fn shutdown(&self) {
        let mut guard = self.inner.lock().unwrap();
        if let Some(mut r) = guard.whisper.take() {
            r.kill();
        }
        if let Some(mut r) = guard.llama.take() {
            r.kill();
        }
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

/// Block until the server is ready, or it dies / times out.
///
/// TCP-accept alone is not enough for llama.cpp: it binds the port *before* the
/// model finishes loading into VRAM, so an early request races ahead and gets
/// `503 {"error":{"message":"Loading model"}}`. When `health_path` is set we
/// additionally poll that HTTP route until it returns `200` (llama's `/health`
/// returns `503` while loading, `200` when ready). whisper.cpp loads its model
/// before binding, so TCP-accept is sufficient there (`health_path = None`).
fn wait_ready(r: &mut Running, health_path: Option<&str>) -> Result<(), String> {
    let addr: std::net::SocketAddr = format!("127.0.0.1:{}", r.port)
        .parse()
        .map_err(|e| format!("bad addr: {e}"))?;
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
        if let Ok(Some(status)) = r.child.try_wait() {
            return Err(format!("server exited during startup ({status})"));
        }
        if TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok() {
            match health_path {
                None => return Ok(()),
                Some(path) if http_status(&addr, path) == Some(200) => return Ok(()),
                Some(_) => {} // listening but model still loading — keep waiting
            }
        }
        if Instant::now() >= deadline {
            r.kill();
            return Err("server did not start listening in time".into());
        }
        std::thread::sleep(Duration::from_millis(300));
    }
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
