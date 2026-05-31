//! Minimal 16-bit PCM mono WAV encoder, shared by the STT provider (upload
//! payload) and the recording orchestrator (on-disk clips when `saveAudio` is on).

/// Encode mono `f32` PCM (range -1.0..=1.0) as a 16-bit PCM WAV byte buffer.
pub fn encode_wav_pcm16(pcm: &[f32], sample_rate: u32) -> Vec<u8> {
    let bytes_per_sample = 2u32;
    let data_len = pcm.len() as u32 * bytes_per_sample;
    let byte_rate = sample_rate * bytes_per_sample;
    let mut buf = Vec::with_capacity(44 + data_len as usize);

    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&(36 + data_len).to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes()); // PCM fmt chunk size
    buf.extend_from_slice(&1u16.to_le_bytes()); // audio format = PCM
    buf.extend_from_slice(&1u16.to_le_bytes()); // channels = mono
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&byte_rate.to_le_bytes());
    buf.extend_from_slice(&(bytes_per_sample as u16).to_le_bytes()); // block align
    buf.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_len.to_le_bytes());
    for &s in pcm {
        let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        buf.extend_from_slice(&v.to_le_bytes());
    }
    buf
}
