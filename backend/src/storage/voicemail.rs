use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use tokio::fs;

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
        .map(|c| if matches!(c, '/' | '\\' | ':') { '_' } else { c })
        .collect()
}

pub fn pcm16_wav_bytes(samples: &[i16], sample_rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes());
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
            if offset + len > bytes.len() {
                return Err(anyhow!("truncated WAV data"));
            }
            let mut samples = Vec::with_capacity(len / 2);
            for chunk in bytes[offset..offset + len].chunks_exact(2) {
                samples.push(i16::from_le_bytes([chunk[0], chunk[1]]));
            }
            return Ok(DecodedWav {
                sample_rate,
                samples,
            });
        }
        offset += len + (len % 2);
    }
    Err(anyhow!("WAV data chunk not found"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn wav_round_trip_preserves_samples() {
        let samples = vec![-32768, -123, 0, 123, 32767];
        let wav = pcm16_wav_bytes(&samples, 8000);
        let decoded = read_pcm16_wav(&wav).expect("valid wav");
        assert_eq!(decoded.sample_rate, 8000);
        assert_eq!(decoded.samples, samples);
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
        let _ = fs::remove_dir_all(root);
    }
}
