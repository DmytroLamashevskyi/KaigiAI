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
/// (esp. whisper large / big GGUF) can take a while.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(90);

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
                return Ok(base_url(r.port));
            }
            r.kill();
            guard.whisper = None;
        }
        let port = pick_port(DEFAULT_WHISPER_PORT);
        let child = spawn(&exe, &["-m", &model, "--host", "127.0.0.1", "--port", &port.to_string()])?;
        let mut running = Running { child, sig, port };
        wait_ready(&mut running)?;
        guard.whisper = Some(running);
        Ok(base_url(port))
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
        wait_ready(&mut running)?;
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

/// Block until the server is accepting TCP connections, or it dies / times out.
fn wait_ready(r: &mut Running) -> Result<(), String> {
    let addr = format!("127.0.0.1:{}", r.port);
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
        if let Ok(Some(status)) = r.child.try_wait() {
            return Err(format!("server exited during startup ({status})"));
        }
        if TcpStream::connect_timeout(
            &addr.parse().map_err(|e| format!("bad addr: {e}"))?,
            Duration::from_millis(500),
        )
        .is_ok()
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            r.kill();
            return Err("server did not start listening in time".into());
        }
        std::thread::sleep(Duration::from_millis(300));
    }
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
