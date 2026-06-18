//! Microphone capture via cpal.
//!
//! Opens an input device, downmixes to mono, resamples to 16 kHz, and runs the
//! [`Vad`] on a worker thread so the realtime audio callback stays cheap. Each
//! completed speech segment is handed to a user callback.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, StreamConfig};

use super::vad::{Vad, VadConfig, VadEvent, SAMPLE_RATE};

/// Where to capture audio from.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AudioSource {
    /// A microphone / line-in (WASAPI capture endpoint).
    Mic,
    /// What the machine is playing, via WASAPI loopback on the output device —
    /// lets us transcribe the *other* party in a call/meeting.
    System,
}

/// Names of available input devices (best-effort; empty on error).
pub fn list_input_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.input_devices()
        .map(|it| it.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}

/// A running capture session. Dropping it stops the stream and joins the
/// worker, flushing any trailing segment.
pub struct AudioCapture {
    stream: Option<cpal::Stream>,
    worker: Option<JoinHandle<()>>,
}

impl AudioCapture {
    /// Start capturing. For `Mic`, `device_name` selects an input by name
    /// (`None` = default input). For `System`, audio is captured via loopback on
    /// the default output device (`device_name` is ignored). `on_event` is
    /// invoked from the worker thread with each [`VadEvent`] (live pause/abort
    /// signals plus finalized segments of mono 16 kHz samples).
    pub fn start<F>(
        source: AudioSource,
        device_name: Option<&str>,
        vad_cfg: VadConfig,
        on_event: F,
    ) -> Result<Self, String>
    where
        F: FnMut(VadEvent) + Send + 'static,
    {
        // System audio uses the output endpoint in loopback mode; its format must
        // come from `default_output_config`, but we still build an *input* stream.
        let (device, supported) = match source {
            AudioSource::Mic => {
                let device = pick_input_device(device_name)?;
                let cfg = device
                    .default_input_config()
                    .map_err(|e| format!("no default input config: {e}"))?;
                (device, cfg)
            }
            AudioSource::System => {
                let device = default_output_device()?;
                let cfg = device
                    .default_output_config()
                    .map_err(|e| format!("no default output config: {e}"))?;
                (device, cfg)
            }
        };
        let sample_format = supported.sample_format();
        let config: StreamConfig = supported.into();
        let channels = config.channels as usize;
        let in_rate = config.sample_rate.0;

        // Audio thread -> worker thread: mono samples at the device rate.
        let (tx, rx): (Sender<Vec<f32>>, Receiver<Vec<f32>>) = mpsc::channel();

        let worker = spawn_worker(rx, in_rate, vad_cfg, on_event);

        let err_fn = |e| log::error!("audio stream error: {e}");
        let stream = match sample_format {
            SampleFormat::F32 => build_stream::<f32>(&device, &config, channels, tx, err_fn),
            SampleFormat::I16 => build_stream::<i16>(&device, &config, channels, tx, err_fn),
            SampleFormat::U16 => build_stream::<u16>(&device, &config, channels, tx, err_fn),
            other => Err(format!("unsupported sample format: {other:?}")),
        }?;

        stream.play().map_err(|e| format!("failed to start stream: {e}"))?;

        Ok(Self {
            stream: Some(stream),
            worker: Some(worker),
        })
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        // Drop the stream first: this stops the callback and closes the sender
        // captured inside it, so the worker's recv() ends and it can flush.
        self.stream.take();
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

fn pick_input_device(name: Option<&str>) -> Result<Device, String> {
    let host = cpal::default_host();
    match name.filter(|n| !n.is_empty()) {
        Some(want) => host
            .input_devices()
            .map_err(|e| e.to_string())?
            .find(|d| d.name().map(|n| n == want).unwrap_or(false))
            .ok_or_else(|| format!("input device '{want}' not found")),
        None => host
            .default_input_device()
            .ok_or_else(|| "no default input device".to_string()),
    }
}

/// Default output (render) endpoint — used as the loopback source for system audio.
fn default_output_device() -> Result<Device, String> {
    cpal::default_host()
        .default_output_device()
        .ok_or_else(|| "no default output device for system-audio capture".to_string())
}

/// Build a typed input stream that downmixes to mono and forwards samples.
fn build_stream<T>(
    device: &Device,
    config: &StreamConfig,
    channels: usize,
    tx: Sender<Vec<f32>>,
    err_fn: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream, String>
where
    T: cpal::Sample + cpal::SizedSample + ToF32,
{
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                let mono = downmix_mono(data, channels);
                // Worker gone (capture stopping) -> ignore.
                let _ = tx.send(mono);
            },
            err_fn,
            None,
        )
        .map_err(|e| format!("failed to build input stream: {e}"))
}

fn spawn_worker<F>(
    rx: Receiver<Vec<f32>>,
    in_rate: u32,
    vad_cfg: VadConfig,
    mut on_event: F,
) -> JoinHandle<()>
where
    F: FnMut(VadEvent) + Send + 'static,
{
    thread::spawn(move || {
        let mut vad = Vad::new(vad_cfg);
        let mut resampler = LinearResampler::new(in_rate, SAMPLE_RATE);
        while let Ok(chunk) = rx.recv() {
            let resampled = resampler.process(&chunk);
            for event in vad.push(&resampled) {
                on_event(event);
            }
        }
        // Capture stopped: emit any in-progress utterance (or abort a showing
        // countdown bar) so no placeholder is left orphaned.
        for event in vad.flush() {
            on_event(event);
        }
    })
}

fn downmix_mono<T: ToF32>(data: &[T], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return data.iter().map(|s| s.to_f32()).collect();
    }
    data.chunks(channels)
        .map(|frame| frame.iter().map(|s| s.to_f32()).sum::<f32>() / channels as f32)
        .collect()
}

/// Minimal linear-interpolation resampler. Quality is adequate for VAD and
/// whisper (which is robust to mild artifacts) and avoids a heavy dependency.
struct LinearResampler {
    ratio: f64, // input samples per output sample
    pos: f64,
    last: f32,
    primed: bool,
}

impl LinearResampler {
    fn new(in_rate: u32, out_rate: u32) -> Self {
        Self {
            ratio: in_rate as f64 / out_rate as f64,
            pos: 0.0,
            last: 0.0,
            primed: false,
        }
    }

    fn process(&mut self, input: &[f32]) -> Vec<f32> {
        if input.is_empty() {
            return Vec::new();
        }
        // Pass-through when rates already match.
        if (self.ratio - 1.0).abs() < f64::EPSILON {
            return input.to_vec();
        }
        let mut out = Vec::with_capacity((input.len() as f64 / self.ratio) as usize + 1);
        // `pos` is a position in a virtual stream where index 0 is `last` (the
        // final sample of the previous chunk) and index 1.. are `input`.
        if !self.primed {
            self.last = input[0];
            self.primed = true;
        }
        while self.pos < input.len() as f64 {
            let i = self.pos.floor() as isize;
            let frac = (self.pos - self.pos.floor()) as f32;
            let a = if i < 0 { self.last } else { input[i as usize] };
            let b = if (i + 1) < input.len() as isize {
                input[(i + 1) as usize]
            } else {
                input[input.len() - 1]
            };
            out.push(a + (b - a) * frac);
            self.pos += self.ratio;
        }
        self.pos -= input.len() as f64;
        self.last = input[input.len() - 1];
        out
    }
}

/// Convert cpal's native sample types into normalized `f32` in -1.0..=1.0.
trait ToF32 {
    fn to_f32(&self) -> f32;
}
impl ToF32 for f32 {
    fn to_f32(&self) -> f32 {
        *self
    }
}
impl ToF32 for i16 {
    fn to_f32(&self) -> f32 {
        *self as f32 / i16::MAX as f32
    }
}
impl ToF32 for u16 {
    fn to_f32(&self) -> f32 {
        (*self as f32 / u16::MAX as f32) * 2.0 - 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampler_halves_rate_roughly() {
        let mut r = LinearResampler::new(32_000, 16_000);
        let input = vec![0.0f32; 3200]; // 100 ms at 32 kHz
        let out = r.process(&input);
        // ~1600 samples at 16 kHz, allow a couple off for edge handling.
        assert!((out.len() as isize - 1600).abs() <= 2, "got {}", out.len());
    }

    #[test]
    fn resampler_passthrough_when_equal() {
        let mut r = LinearResampler::new(16_000, 16_000);
        let input = vec![0.1, 0.2, 0.3];
        assert_eq!(r.process(&input), input);
    }

    #[test]
    fn downmix_averages_stereo() {
        let interleaved = [1.0f32, 0.0, 0.0, 1.0];
        assert_eq!(downmix_mono(&interleaved, 2), vec![0.5, 0.5]);
    }
}
