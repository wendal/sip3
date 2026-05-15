use anyhow::{Context, Result, anyhow};
use std::path::{Path, PathBuf};
use tokio::fs;

const MAX_WAV_SAMPLES: usize = 10_000_000;

#[derive(Debug, Clone)]
pub struct DecodedWav {
    pub sample_rate: u32,
    pub samples: Vec<i16>,
}

#[derive(Debug, Clone)]
pub struct LocalVoicemailStorage {
    root: PathBuf,
}

impl LocalVoicemailStorage {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn path_for_key(&self, key: &str) -> Result<PathBuf> {
        if key.contains("..") || Path::new(key).is_absolute() {
            return Err(anyhow!("invalid voicemail storage key"));
        }
        Ok(self.root.join(key))
    }

    pub async fn write_message(
        &self,
        mailbox: &str,
        call_id: &str,
        bytes: &[u8],
    ) -> Result<String> {
        let mailbox = sanitize_key_part(mailbox);
        let call_id = sanitize_key_part(call_id);
        let key = format!("{}/{}.wav", mailbox, call_id);
        let path = self.path_for_key(&key)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(&path, bytes)
            .await
            .with_context(|| format!("write voicemail {:?}", path))?;
        Ok(key)
    }

    pub async fn read(&self, key: &str) -> Result<Vec<u8>> {
        let path = self.path_for_key(key)?;
        fs::read(&path)
            .await
            .with_context(|| format!("read voicemail {:?}", path))
    }

    pub async fn delete(&self, key: &str) -> Result<()> {
        let path = self.path_for_key(key)?;
        match fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e).with_context(|| format!("delete voicemail {:?}", path)),
        }
    }
}

fn sanitize_key_part(input: &str) -> String {
    input
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '\0') {
                '_'
            } else {
                c
            }
        })
        .collect()
}

pub fn pcm16_wav_bytes(samples: &[i16], sample_rate: u32) -> Vec<u8> {
    if sample_rate == 0 {
        panic!("sample_rate must be non-zero");
    }

    let num_samples = samples.len();
    let data_len = num_samples
        .checked_mul(2)
        .and_then(|v| u32::try_from(v).ok())
        .expect("WAV data size exceeds u32 limit");

    let byte_rate = sample_rate
        .checked_mul(2)
        .expect("sample_rate * 2 overflows u32");

    let riff_chunk_size = 36u32
        .checked_add(data_len)
        .expect("RIFF chunk size exceeds u32 limit");

    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_chunk_size.to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

pub fn read_pcm16_wav(bytes: &[u8]) -> Result<DecodedWav> {
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(anyhow!("not a WAV file"));
    }
    let audio_format = u16::from_le_bytes([bytes[20], bytes[21]]);
    let channels = u16::from_le_bytes([bytes[22], bytes[23]]);
    let sample_rate = u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]);
    let bits_per_sample = u16::from_le_bytes([bytes[34], bytes[35]]);
    if audio_format != 1 || channels != 1 || bits_per_sample != 16 {
        return Err(anyhow!("WAV must be mono PCM16"));
    }
    let mut offset = 12usize;
    while offset + 8 <= bytes.len() {
        let id = &bytes[offset..offset + 4];
        let len = u32::from_le_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]) as usize;
        offset += 8;
        if id == b"data" {
            let data_end = offset
                .checked_add(len)
                .ok_or_else(|| anyhow!("truncated WAV data"))?;
            if data_end > bytes.len() {
                return Err(anyhow!("truncated WAV data"));
            }
            let num_samples = len / 2;
            if num_samples > MAX_WAV_SAMPLES {
                return Err(anyhow!("WAV data exceeds maximum sample limit"));
            }
            let mut samples = Vec::with_capacity(num_samples);
            for chunk in bytes[offset..data_end].chunks_exact(2) {
                samples.push(i16::from_le_bytes([chunk[0], chunk[1]]));
            }
            return Ok(DecodedWav {
                sample_rate,
                samples,
            });
        }
        // Skip non-data chunks with padding
        let padding = len % 2;
        offset = offset
            .checked_add(len)
            .and_then(|o| o.checked_add(padding))
            .ok_or_else(|| anyhow!("WAV chunk offset overflow"))?;
        if offset > bytes.len() {
            return Err(anyhow!("WAV chunk extends past file end"));
        }
    }
    Err(anyhow!("WAV data chunk not found"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_round_trip_preserves_samples() {
        let samples = vec![-32768, -123, 0, 123, 32767];
        let wav = pcm16_wav_bytes(&samples, 8000);
        let decoded = read_pcm16_wav(&wav).expect("valid wav");
        assert_eq!(decoded.sample_rate, 8000);
        assert_eq!(decoded.samples, samples);
    }

    #[test]
    fn wav_empty_samples_round_trip() {
        let samples: Vec<i16> = vec![];
        let wav = pcm16_wav_bytes(&samples, 8000);
        let decoded = read_pcm16_wav(&wav).expect("valid empty wav");
        assert_eq!(decoded.sample_rate, 8000);
        assert_eq!(decoded.samples, samples);
    }

    #[test]
    #[should_panic(expected = "sample_rate must be non-zero")]
    fn wav_zero_sample_rate_panics() {
        let samples = vec![100i16];
        pcm16_wav_bytes(&samples, 0);
    }

    #[test]
    fn read_pcm16_wav_rejects_empty_input() {
        let result = read_pcm16_wav(&[]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not a WAV file"));
    }

    #[test]
    fn read_pcm16_wav_rejects_truncated_header() {
        let truncated = b"RIFF\x00\x00\x00\x00WAVE";
        let result = read_pcm16_wav(truncated);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not a WAV file"));
    }

    #[test]
    fn read_pcm16_wav_rejects_stereo() {
        let mut wav = pcm16_wav_bytes(&[100i16], 8000);
        // Change channel count from 1 to 2 at offset 22-23
        wav[22] = 2;
        wav[23] = 0;
        let result = read_pcm16_wav(&wav);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("WAV must be mono PCM16")
        );
    }

    #[test]
    fn read_pcm16_wav_rejects_malformed_chunk_overflow() {
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&100u32.to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // audio_format = 1
        wav.extend_from_slice(&1u16.to_le_bytes()); // channels = 1
        wav.extend_from_slice(&8000u32.to_le_bytes()); // sample_rate
        wav.extend_from_slice(&16000u32.to_le_bytes()); // byte_rate
        wav.extend_from_slice(&2u16.to_le_bytes()); // block_align
        wav.extend_from_slice(&16u16.to_le_bytes()); // bits_per_sample
        // Add a non-data chunk with a length that extends past the file
        wav.extend_from_slice(b"junk");
        wav.extend_from_slice(&0xFFFFFFFFu32.to_le_bytes()); // huge length
        let result = read_pcm16_wav(&wav);
        assert!(result.is_err());
        let err_str = result.unwrap_err().to_string();
        assert!(
            err_str.contains("chunk extends past file end") || err_str.contains("overflow"),
            "got error: {}",
            err_str
        );
    }

    #[test]
    fn read_pcm16_wav_rejects_data_chunk_past_eof() {
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&100u32.to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // audio_format = 1
        wav.extend_from_slice(&1u16.to_le_bytes()); // channels = 1
        wav.extend_from_slice(&8000u32.to_le_bytes()); // sample_rate
        wav.extend_from_slice(&16000u32.to_le_bytes()); // byte_rate
        wav.extend_from_slice(&2u16.to_le_bytes()); // block_align
        wav.extend_from_slice(&16u16.to_le_bytes()); // bits_per_sample
        // Add data chunk with length that claims more data than available
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&1000u32.to_le_bytes()); // claims 1000 bytes but file ends here
        let result = read_pcm16_wav(&wav);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("truncated WAV data")
        );
    }

    #[test]
    fn sanitize_key_part_removes_dangerous_chars() {
        assert_eq!(sanitize_key_part("foo/bar"), "foo_bar");
        assert_eq!(sanitize_key_part("foo\\bar"), "foo_bar");
        assert_eq!(sanitize_key_part("C:Users"), "C_Users");
        assert_eq!(sanitize_key_part("foo\0bar"), "foo_bar");
        assert_eq!(sanitize_key_part("safe-123.wav"), "safe-123.wav");
    }

    #[test]
    fn path_for_key_rejects_traversal() {
        let storage = LocalVoicemailStorage::new(PathBuf::from("/tmp/vm"));
        assert!(storage.path_for_key("../etc/passwd").is_err());
        assert!(storage.path_for_key("foo/../bar").is_err());
    }

    #[test]
    #[cfg(unix)]
    fn path_for_key_rejects_absolute_unix() {
        let storage = LocalVoicemailStorage::new(PathBuf::from("/tmp/vm"));
        assert!(storage.path_for_key("/etc/passwd").is_err());
    }

    #[test]
    #[cfg(windows)]
    fn path_for_key_rejects_absolute_windows() {
        let storage = LocalVoicemailStorage::new(PathBuf::from("C:\\voicemail"));
        assert!(
            storage
                .path_for_key("C:\\Windows\\System32\\config")
                .is_err()
        );
        assert!(storage.path_for_key("D:\\data").is_err());
    }

    #[tokio::test]
    async fn local_storage_writes_and_reads_message() {
        let root = std::env::temp_dir().join(format!("sip3-vm-test-{}", rand::random::<u64>()));
        let storage = LocalVoicemailStorage::new(root.clone());
        let key = storage
            .write_message("1001", "call-a", b"hello")
            .await
            .expect("write");
        assert!(key.ends_with(".wav"));
        assert_eq!(storage.read(&key).await.expect("read"), b"hello");
        storage.delete(&key).await.expect("delete");
        assert!(!root.join(&key).exists());
        let _ = fs::remove_dir_all(root).await;
    }
}
