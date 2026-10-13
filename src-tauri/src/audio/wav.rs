//! Minimal mono WAV encoder (16-bit PCM), shared by the STT provider (upload
//! payload) and the recording orchestrator (on-disk clips when `saveAudio` is on),
//! plus a decoder for the benchmark.

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

/// Decode a mono WAV (16-bit PCM or 32-bit float) into `f32` samples and its
/// sample rate. Used by the benchmark (§10.19) to read FLEURS clips; walks the
/// RIFF chunks so extra chunks (LIST, fact, …) before `data` are skipped.
pub fn decode_wav_mono(bytes: &[u8]) -> Result<(Vec<f32>, u32), String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file".into());
    }
    let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    let mut fmt: Option<(u16, u16, u32, u16)> = None; // format, channels, rate, bits
    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let len = u32_at(pos + 4) as usize;
        let body = pos + 8;
        let end = body.saturating_add(len).min(bytes.len());
        if id == b"fmt " && len >= 16 {
            fmt = Some((u16_at(body), u16_at(body + 2), u32_at(body + 4), u16_at(body + 14)));
        } else if id == b"data" {
            let (format, channels, rate, bits) = fmt.ok_or("data chunk before fmt chunk")?;
            if channels != 1 {
                return Err(format!("expected mono, got {channels} channels"));
            }
            let data = &bytes[body..end];
            let pcm = match (format, bits) {
                (1, 16) => data
                    .chunks_exact(2)
                    .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / i16::MAX as f32)
                    .collect(),
                (3, 32) => data
                    .chunks_exact(4)
                    .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                    .collect(),
                _ => return Err(format!("unsupported WAV format {format} / {bits}-bit")),
            };
            return Ok((pcm, rate));
        }
        // Chunks are word-aligned: odd lengths carry a pad byte.
        pos = body + len + (len & 1);
    }
    Err("no data chunk".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm16_roundtrip() {
        let pcm = vec![0.0, 0.5, -0.5, 0.25];
        let (back, rate) = decode_wav_mono(&encode_wav_pcm16(&pcm, 16_000)).unwrap();
        assert_eq!(rate, 16_000);
        assert_eq!(back.len(), pcm.len());
        for (a, b) in pcm.iter().zip(&back) {
            assert!((a - b).abs() < 1e-3, "{a} vs {b}");
        }
    }

    #[test]
    fn float32_is_read() {
        let mut wav = encode_wav_pcm16(&[], 16_000);
        wav[20..22].copy_from_slice(&3u16.to_le_bytes()); // IEEE float
        wav[34..36].copy_from_slice(&32u16.to_le_bytes());
        wav.truncate(40);
        wav.extend_from_slice(&8u32.to_le_bytes());
        wav.extend_from_slice(&0.75f32.to_le_bytes());
        wav.extend_from_slice(&(-1.0f32).to_le_bytes());
        assert_eq!(decode_wav_mono(&wav).unwrap(), (vec![0.75, -1.0], 16_000));
    }
}
