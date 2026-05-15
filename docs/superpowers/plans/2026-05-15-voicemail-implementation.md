# Voicemail Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build Linphone-compatible voicemail for SIP3: offline/no-answer recording, `*97` mailbox playback, and message-summary MWI.

**Architecture:** Implement voicemail as a local SIP B2BUA endpoint that can answer calls locally and terminate RTP. Reuse the existing SIP parser/response helpers, existing G.711 helpers, and the same local-endpoint wiring style used by conference rooms. Store voicemail metadata in MySQL and audio as local WAV files behind a storage trait designed for future object storage.

**Tech Stack:** Rust 1.95, Tokio UDP/tasks, Axum, SQLx MySQL, Vue 3, Element Plus, SIP RTP/AVP G.711 PCMU/PCMA, WAV PCM16/8kHz.

---

## File structure

- Create `migrations\012_voicemail.sql` and `backend\migrations\012_voicemail.sql`: voicemail boxes, messages, and MWI subscriptions.
- Modify `backend\src\config.rs`: voicemail extension, storage/prompt paths, RTP range, timeouts.
- Create `backend\src\models\voicemail.rs`; modify `backend\src\models\mod.rs`: typed rows and request DTOs.
- Create `backend\src\storage\mod.rs` and `backend\src\storage\voicemail.rs`; modify `backend\src\lib.rs`: storage abstraction and local filesystem implementation.
- Create `backend\src\sip\voicemail_sdp.rs`: G.711 SDP negotiation and answer generation for voicemail.
- Create `backend\src\sip\voicemail_mwi.rs`: message-summary subscription persistence and NOTIFY body/build/send helpers.
- Create `backend\src\sip\voicemail_media.rs`: RTP recording/playback, WAV read/write helpers, DTMF parsing.
- Create `backend\src\sip\voicemail.rs`: mailbox lookup, `*97` access endpoint, offline delivery endpoint, no-answer session coordinator.
- Modify `backend\src\sip\mod.rs`: export voicemail modules.
- Modify `backend\src\sip\handler.rs`: own `Voicemail`, route `*97`, route active voicemail dialog requests, cancel timers on final responses, expose accessors.
- Modify `backend\src\sip\proxy.rs`: invoke offline voicemail when callee is unregistered, schedule no-answer voicemail after forwarding an INVITE.
- Modify `backend\src\sip\server.rs`: reconcile/cleanup voicemail state on startup and expose voicemail RTP Docker/firewall requirements in docs.
- Create `backend\src\api\voicemail.rs`; modify `backend\src\api\mod.rs`: admin APIs for boxes and messages.
- Create `frontend\src\views\Voicemail.vue`; modify `frontend\src\router\index.js` and `frontend\src\components\SideNav.vue`: admin UI.
- Modify `README.md`, `docs\deployment.md`, and `docker-compose.yml`: document config, dialing, MWI, storage, prompt files, RTP range.

## Task 1: Schema, config, and model types

**Files:**
- Create: `migrations\012_voicemail.sql`
- Create: `backend\migrations\012_voicemail.sql`
- Create: `backend\src\models\voicemail.rs`
- Modify: `backend\src\models\mod.rs`
- Modify: `backend\src\config.rs`

- [ ] **Step 1: Write model validation tests first**

Add this test module to the bottom of `backend\src\models\voicemail.rs` when creating the file:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_message_statuses() {
        assert!(validate_voicemail_status("new").is_ok());
        assert!(validate_voicemail_status("saved").is_ok());
        assert!(validate_voicemail_status("deleted").is_ok());
        assert!(validate_voicemail_status("heard").is_err());
    }

    #[test]
    fn validates_box_limits() {
        assert!(validate_box_limits(25, 120, 100).is_ok());
        assert!(validate_box_limits(0, 120, 100).is_err());
        assert!(validate_box_limits(25, 0, 100).is_err());
        assert!(validate_box_limits(25, 120, 0).is_err());
        assert!(validate_box_limits(601, 120, 100).is_err());
    }
}
```

- [ ] **Step 2: Run the focused test and verify it fails**

Run:

```powershell
cd backend
cargo test models::voicemail --lib
```

Expected: FAIL because `models::voicemail` does not exist.

- [ ] **Step 3: Add migrations**

Create identical content in both `migrations\012_voicemail.sql` and `backend\migrations\012_voicemail.sql`:

```sql
CREATE TABLE IF NOT EXISTS sip_voicemail_boxes (
    id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
    username VARCHAR(64) NOT NULL,
    domain VARCHAR(128) NOT NULL,
    enabled TINYINT(1) NOT NULL DEFAULT 1,
    no_answer_secs INT UNSIGNED NOT NULL DEFAULT 25,
    max_message_secs INT UNSIGNED NOT NULL DEFAULT 120,
    max_messages INT UNSIGNED NOT NULL DEFAULT 100,
    greeting_storage_key VARCHAR(512) NULL,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
    UNIQUE KEY uniq_voicemail_box (username, domain),
    CONSTRAINT fk_voicemail_box_account
      FOREIGN KEY (username, domain)
      REFERENCES sip_accounts(username, domain)
      ON DELETE CASCADE
      ON UPDATE CASCADE
);

CREATE TABLE IF NOT EXISTS sip_voicemail_messages (
    id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
    box_id BIGINT UNSIGNED NOT NULL,
    caller VARCHAR(128) NOT NULL,
    callee VARCHAR(128) NOT NULL,
    call_id VARCHAR(255) NOT NULL,
    duration_secs INT UNSIGNED NOT NULL DEFAULT 0,
    storage_key VARCHAR(512) NOT NULL,
    content_type VARCHAR(128) NOT NULL DEFAULT 'audio/wav',
    status VARCHAR(32) NOT NULL DEFAULT 'new',
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    heard_at DATETIME NULL,
    KEY idx_voicemail_box_status_created (box_id, status, created_at),
    KEY idx_voicemail_call_id (call_id),
    CONSTRAINT fk_voicemail_message_box
      FOREIGN KEY (box_id)
      REFERENCES sip_voicemail_boxes(id)
      ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS sip_voicemail_mwi_subscriptions (
    id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
    subscriber VARCHAR(64) NOT NULL,
    domain VARCHAR(128) NOT NULL,
    call_id VARCHAR(255) NOT NULL,
    subscriber_tag VARCHAR(128) NOT NULL,
    subscriber_ip VARCHAR(45) NOT NULL,
    subscriber_port SMALLINT UNSIGNED NOT NULL,
    expires_at DATETIME NOT NULL,
    cseq INT UNSIGNED NOT NULL DEFAULT 1,
    UNIQUE KEY uniq_voicemail_mwi_subscription (subscriber, domain, call_id),
    KEY idx_voicemail_mwi_expires (expires_at)
);
```

- [ ] **Step 4: Add model structs and validation helpers**

Create `backend\src\models\voicemail.rs`:

```rust
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

pub const VOICEMAIL_STATUS_NEW: &str = "new";
pub const VOICEMAIL_STATUS_SAVED: &str = "saved";
pub const VOICEMAIL_STATUS_DELETED: &str = "deleted";

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct VoicemailBox {
    pub id: u64,
    pub username: String,
    pub domain: String,
    pub enabled: i8,
    pub no_answer_secs: u32,
    pub max_message_secs: u32,
    pub max_messages: u32,
    pub greeting_storage_key: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct VoicemailBoxSummary {
    pub id: u64,
    pub username: String,
    pub domain: String,
    pub enabled: i8,
    pub no_answer_secs: u32,
    pub max_message_secs: u32,
    pub max_messages: u32,
    pub new_count: i64,
    pub saved_count: i64,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct VoicemailMessage {
    pub id: u64,
    pub box_id: u64,
    pub caller: String,
    pub callee: String,
    pub call_id: String,
    pub duration_secs: u32,
    pub storage_key: String,
    pub content_type: String,
    pub status: String,
    pub created_at: NaiveDateTime,
    pub heard_at: Option<NaiveDateTime>,
}

#[derive(Debug, Deserialize)]
pub struct CreateVoicemailBox {
    pub username: String,
    pub domain: Option<String>,
    pub enabled: Option<i8>,
    pub no_answer_secs: Option<u32>,
    pub max_message_secs: Option<u32>,
    pub max_messages: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateVoicemailBox {
    pub enabled: Option<i8>,
    pub no_answer_secs: Option<u32>,
    pub max_message_secs: Option<u32>,
    pub max_messages: Option<u32>,
    pub greeting_storage_key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateVoicemailMessage {
    pub status: String,
}

pub fn validate_voicemail_status(status: &str) -> Result<(), &'static str> {
    match status {
        VOICEMAIL_STATUS_NEW | VOICEMAIL_STATUS_SAVED | VOICEMAIL_STATUS_DELETED => Ok(()),
        _ => Err("voicemail status must be one of: new, saved, deleted"),
    }
}

pub fn validate_box_limits(
    no_answer_secs: u32,
    max_message_secs: u32,
    max_messages: u32,
) -> Result<(), &'static str> {
    if !(1..=600).contains(&no_answer_secs) {
        return Err("no_answer_secs must be between 1 and 600");
    }
    if !(1..=3600).contains(&max_message_secs) {
        return Err("max_message_secs must be between 1 and 3600");
    }
    if !(1..=10_000).contains(&max_messages) {
        return Err("max_messages must be between 1 and 10000");
    }
    Ok(())
}
```

Modify `backend\src\models\mod.rs`:

```rust
pub mod voicemail;

pub use voicemail::{
    CreateVoicemailBox, UpdateVoicemailBox, UpdateVoicemailMessage, VoicemailBox,
    VoicemailBoxSummary, VoicemailMessage, validate_box_limits, validate_voicemail_status,
};
```

Keep existing module exports in `mod.rs`; add these lines without removing current exports.

- [ ] **Step 5: Add config fields**

In `backend\src\config.rs`, add fields to `ServerConfig`:

```rust
pub voicemail_access_extension: String,
pub voicemail_no_answer_secs: u64,
pub voicemail_max_message_secs: u64,
pub voicemail_idle_timeout_secs: u64,
pub voicemail_storage_dir: String,
pub voicemail_prompt_dir: String,
pub voicemail_rtp_port_min: u16,
pub voicemail_rtp_port_max: u16,
```

In the defaults section inside `Config::load()`, add:

```rust
.set_default("server.voicemail_access_extension", "*97")?
.set_default("server.voicemail_no_answer_secs", 25)?
.set_default("server.voicemail_max_message_secs", 120)?
.set_default("server.voicemail_idle_timeout_secs", 10)?
.set_default("server.voicemail_storage_dir", "voicemail")?
.set_default("server.voicemail_prompt_dir", "voicemail/prompts")?
.set_default("server.voicemail_rtp_port_min", 10200)?
.set_default("server.voicemail_rtp_port_max", 10299)?
```

- [ ] **Step 6: Run model tests**

Run:

```powershell
cd backend
cargo test models::voicemail --lib
```

Expected: PASS.

- [ ] **Step 7: Commit**

```powershell
git add -- migrations\012_voicemail.sql backend\migrations\012_voicemail.sql backend\src\models\voicemail.rs backend\src\models\mod.rs backend\src\config.rs
git commit -m "feat: add voicemail schema and models" -m "Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"
```

## Task 2: Storage trait and WAV helpers

**Files:**
- Create: `backend\src\storage\mod.rs`
- Create: `backend\src\storage\voicemail.rs`
- Modify: `backend\src\lib.rs`

- [ ] **Step 1: Write storage and WAV tests first**

Create `backend\src\storage\voicemail.rs` with the tests first:

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```powershell
cd backend
cargo test storage::voicemail --lib
```

Expected: FAIL because storage module and helpers are not defined.

- [ ] **Step 3: Implement storage module**

Use this public API in `backend\src\storage\voicemail.rs`:

```rust
use anyhow::{Context, Result, anyhow};
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

    pub async fn write_message(&self, mailbox: &str, call_id: &str, bytes: &[u8]) -> Result<String> {
        let mailbox = sanitize_key_part(mailbox);
        let call_id = sanitize_key_part(call_id);
        let key = format!("{}/{}.wav", mailbox, call_id);
        let path = self.path_for_key(&key)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(&path, bytes).await.with_context(|| format!("write voicemail {:?}", path))?;
        Ok(key)
    }

    pub async fn read(&self, key: &str) -> Result<Vec<u8>> {
        let path = self.path_for_key(key)?;
        fs::read(&path).await.with_context(|| format!("read voicemail {:?}", path))
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
```

Use concrete `LocalVoicemailStorage` methods for the MVP. When object storage is added, introduce this enum without changing callers:

```rust
#[derive(Debug, Clone)]
pub enum VoicemailStorageBackend {
    Local(LocalVoicemailStorage),
}
```

Then implement `write_message`, `read`, and `delete` on the enum with `match self`.

- [ ] **Step 4: Implement WAV helpers**

Add these helpers in the same file:

```rust
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
            return Ok(DecodedWav { sample_rate, samples });
        }
        offset += len + (len % 2);
    }
    Err(anyhow!("WAV data chunk not found"))
}
```

- [ ] **Step 5: Export storage module**

Create `backend\src\storage\mod.rs`:

```rust
pub mod voicemail;
```

Modify `backend\src\lib.rs`:

```rust
pub mod storage;
```

Keep existing exports in `lib.rs`.

- [ ] **Step 6: Run tests**

```powershell
cd backend
cargo test storage::voicemail --lib
```

Expected: PASS.

- [ ] **Step 7: Commit**

```powershell
git add -- backend\src\storage\mod.rs backend\src\storage\voicemail.rs backend\src\lib.rs
git commit -m "feat: add voicemail local storage" -m "Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"
```

## Task 3: Voicemail SDP negotiation

**Files:**
- Create: `backend\src\sip\voicemail_sdp.rs`
- Modify: `backend\src\sip\mod.rs`

- [ ] **Step 1: Write SDP tests first**

Create `backend\src\sip\voicemail_sdp.rs` with:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negotiates_linphone_pcmu_offer() {
        let offer = "v=0\r\nm=audio 7078 RTP/AVP 0 8 101\r\na=rtpmap:0 PCMU/8000\r\na=rtpmap:8 PCMA/8000\r\na=rtpmap:101 telephone-event/8000\r\n";
        let n = negotiate_offer(offer).expect("negotiate");
        assert_eq!(n.codec, VoicemailCodec::Pcmu);
        assert_eq!(n.audio_pt, 0);
        assert_eq!(n.telephone_event_pt, Some(101));
    }

    #[test]
    fn rejects_srtp_only_offer() {
        let offer = "v=0\r\nm=audio 7078 RTP/SAVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
        assert!(negotiate_offer(offer).is_err());
    }

    #[test]
    fn builds_answer_with_content_codec() {
        let n = VoicemailNegotiation {
            codec: VoicemailCodec::Pcma,
            audio_pt: 8,
            telephone_event_pt: Some(101),
        };
        let answer = build_answer("203.0.113.10", 10200, &n, 1234);
        assert!(answer.contains("m=audio 10200 RTP/AVP 8 101"));
        assert!(answer.contains("a=rtpmap:8 PCMA/8000"));
        assert!(answer.contains("a=rtpmap:101 telephone-event/8000"));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```powershell
cd backend
cargo test sip::voicemail_sdp --lib
```

Expected: FAIL because the module is not exported.

- [ ] **Step 3: Implement by adapting the conference SDP rules**

Add public types:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoicemailCodec {
    Pcmu,
    Pcma,
}

impl VoicemailCodec {
    pub fn static_pt(self) -> u8 {
        match self {
            VoicemailCodec::Pcmu => 0,
            VoicemailCodec::Pcma => 8,
        }
    }

    pub fn rtpmap(self) -> &'static str {
        match self {
            VoicemailCodec::Pcmu => "PCMU/8000",
            VoicemailCodec::Pcma => "PCMA/8000",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoicemailNegotiation {
    pub codec: VoicemailCodec,
    pub audio_pt: u8,
    pub telephone_event_pt: Option<u8>,
}
```

Implement `negotiate_offer(sdp: &str) -> anyhow::Result<VoicemailNegotiation>` with the same parsing behavior as `conference_sdp`: first active `m=audio`, profile must be `RTP/AVP`, choose offered PCMU first then PCMA, detect dynamic `telephone-event/8000`.

Implement:

```rust
pub fn build_answer(
    public_ip: &str,
    relay_port: u16,
    negotiation: &VoicemailNegotiation,
    session_id: u64,
) -> String {
    let mut pts = negotiation.audio_pt.to_string();
    if let Some(dtmf_pt) = negotiation.telephone_event_pt {
        pts.push_str(&format!(" {}", dtmf_pt));
    }
    let mut sdp = String::new();
    sdp.push_str("v=0\r\n");
    sdp.push_str(&format!(
        "o=sip3 {} {} IN IP4 {}\r\n",
        session_id, session_id, public_ip
    ));
    sdp.push_str("s=SIP3 Voicemail\r\n");
    sdp.push_str(&format!("c=IN IP4 {}\r\n", public_ip));
    sdp.push_str("t=0 0\r\n");
    sdp.push_str(&format!("m=audio {} RTP/AVP {}\r\n", relay_port, pts));
    sdp.push_str(&format!(
        "a=rtpmap:{} {}\r\n",
        negotiation.audio_pt,
        negotiation.codec.rtpmap()
    ));
    if let Some(dtmf_pt) = negotiation.telephone_event_pt {
        sdp.push_str(&format!("a=rtpmap:{} telephone-event/8000\r\n", dtmf_pt));
        sdp.push_str(&format!("a=fmtp:{} 0-15\r\n", dtmf_pt));
    }
    sdp.push_str("a=ptime:20\r\n");
    sdp.push_str("a=sendrecv\r\n");
    sdp
}
```

- [ ] **Step 4: Export module and run tests**

Add to `backend\src\sip\mod.rs`:

```rust
pub mod voicemail_sdp;
```

Run:

```powershell
cd backend
cargo test sip::voicemail_sdp --lib
```

Expected: PASS.

- [ ] **Step 5: Commit**

```powershell
git add -- backend\src\sip\voicemail_sdp.rs backend\src\sip\mod.rs
git commit -m "feat: add voicemail SDP negotiation" -m "Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"
```

## Task 4: MWI subscription and NOTIFY helpers

**Files:**
- Create: `backend\src\sip\voicemail_mwi.rs`
- Modify: `backend\src\sip\mod.rs`

- [ ] **Step 1: Write MWI body tests first**

Create `backend\src\sip\voicemail_mwi.rs` with tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_waiting_body_with_new_and_saved_counts() {
        let body = build_message_summary_body("1001", "sip.air32.cn", 2, 1);
        assert!(body.contains("Messages-Waiting: yes"));
        assert!(body.contains("Message-Account: sip:1001@sip.air32.cn"));
        assert!(body.contains("Voice-Message: 2/1 (0/0)"));
    }

    #[test]
    fn formats_empty_body_without_waiting() {
        let body = build_message_summary_body("1001", "sip.air32.cn", 0, 0);
        assert!(body.contains("Messages-Waiting: no"));
        assert!(body.contains("Voice-Message: 0/0 (0/0)"));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```powershell
cd backend
cargo test sip::voicemail_mwi --lib
```

Expected: FAIL because the module is not exported.

- [ ] **Step 3: Implement MWI service skeleton and body builder**

Add:

```rust
use anyhow::Result;
use chrono::Utc;
use sqlx::MySqlPool;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tracing::{info, warn};

use super::handler::{SipMessage, base_response, extract_uri, uri_username};
use crate::config::Config;

#[derive(Clone)]
pub struct VoicemailMwi {
    pool: MySqlPool,
    cfg: Config,
    socket: Arc<UdpSocket>,
}

impl VoicemailMwi {
    pub fn new(pool: MySqlPool, cfg: Config, socket: Arc<UdpSocket>) -> Self {
        Self { pool, cfg, socket }
    }

    pub async fn handle_subscribe(&self, msg: &SipMessage, src: SocketAddr) -> Result<String> {
        let event = msg.header("event").unwrap_or("").to_lowercase();
        let event = event.split(';').next().unwrap_or("").trim();
        if event != "message-summary" {
            return Ok(base_response(msg, 489, "Bad Event").build());
        }

        let from = msg.from_header().unwrap_or("");
        let from_uri = extract_uri(from).unwrap_or_default();
        let subscriber = uri_username(&from_uri).unwrap_or_default();
        if subscriber.is_empty() {
            return Ok(base_response(msg, 400, "Bad Request").build());
        }

        let domain = self.cfg.server.sip_domain.clone();
        let call_id = msg.call_id().unwrap_or("").to_string();
        let subscriber_tag = extract_param(from, "tag").unwrap_or_default();
        let expires = msg.expires().unwrap_or(3600);

        if expires == 0 {
            sqlx::query(
                "DELETE FROM sip_voicemail_mwi_subscriptions WHERE subscriber = ? AND domain = ? AND call_id = ?",
            )
            .bind(&subscriber)
            .bind(&domain)
            .bind(&call_id)
            .execute(&self.pool)
            .await?;
            return Ok(base_response(msg, 200, "OK").build());
        }

        let expires_at = (Utc::now() + chrono::Duration::seconds(i64::from(expires))).naive_utc();
        sqlx::query(
            "INSERT INTO sip_voicemail_mwi_subscriptions
               (subscriber, domain, call_id, subscriber_tag, subscriber_ip, subscriber_port, expires_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON DUPLICATE KEY UPDATE
               subscriber_tag = VALUES(subscriber_tag),
               subscriber_ip = VALUES(subscriber_ip),
               subscriber_port = VALUES(subscriber_port),
               expires_at = VALUES(expires_at),
               cseq = cseq + 1",
        )
        .bind(&subscriber)
        .bind(&domain)
        .bind(&call_id)
        .bind(&subscriber_tag)
        .bind(src.ip().to_string())
        .bind(src.port())
        .bind(expires_at)
        .execute(&self.pool)
        .await?;

        self.notify_mailbox(&subscriber, &domain).await?;
        Ok(base_response(msg, 200, "OK").build())
    }

    pub async fn notify_mailbox(&self, username: &str, domain: &str) -> Result<()> {
        let (new_count, saved_count) = self.message_counts(username, domain).await?;
        let rows: Vec<(String, String, String, u16, u32)> = sqlx::query_as(
            "SELECT call_id, subscriber_tag, subscriber_ip, subscriber_port, cseq
             FROM sip_voicemail_mwi_subscriptions
             WHERE subscriber = ? AND domain = ? AND expires_at > NOW()",
        )
        .bind(username)
        .bind(domain)
        .fetch_all(&self.pool)
        .await?;

        for (call_id, tag, ip, port, cseq) in rows {
            let notify = build_notify(username, domain, &call_id, &tag, cseq + 1, new_count, saved_count);
            let addr: SocketAddr = format!("{}:{}", ip, port).parse()?;
            if let Err(e) = self.socket.send_to(notify.as_bytes(), addr).await {
                warn!("Failed to send voicemail MWI NOTIFY to {}: {}", addr, e);
            } else {
                info!("Sent voicemail MWI NOTIFY to {}", addr);
            }
        }
        Ok(())
    }

    async fn message_counts(&self, username: &str, domain: &str) -> Result<(i64, i64)> {
        let row: Option<(i64, i64)> = sqlx::query_as(
            "SELECT
               (SELECT COUNT(*) FROM sip_voicemail_messages m
                WHERE m.box_id = b.id AND m.status = 'new') AS new_count,
               (SELECT COUNT(*) FROM sip_voicemail_messages m
                WHERE m.box_id = b.id AND m.status = 'saved') AS saved_count
             FROM sip_voicemail_boxes b
             WHERE b.username = ? AND b.domain = ?",
        )
        .bind(username)
        .bind(domain)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.unwrap_or((0, 0)))
    }
}

pub fn build_message_summary_body(username: &str, domain: &str, new_count: i64, saved_count: i64) -> String {
    let waiting = if new_count > 0 { "yes" } else { "no" };
    format!(
        "Messages-Waiting: {}\r\nMessage-Account: sip:{}@{}\r\nVoice-Message: {}/{} (0/0)\r\n",
        waiting, username, domain, new_count, saved_count
    )
}
```

Add helper:

```rust
fn build_notify(
    username: &str,
    domain: &str,
    call_id: &str,
    subscriber_tag: &str,
    cseq: u32,
    new_count: i64,
    saved_count: i64,
) -> String {
    let body = build_message_summary_body(username, domain, new_count, saved_count);
    format!(
        "NOTIFY sip:{}@{} SIP/2.0\r\nVia: SIP/2.0/UDP {};branch=z9hG4bK-vm-{}\r\nFrom: <sip:{}@{}>;tag=sip3-mwi\r\nTo: <sip:{}@{}>;tag={}\r\nCall-ID: {}\r\nCSeq: {} NOTIFY\r\nEvent: message-summary\r\nSubscription-State: active\r\nContent-Type: application/simple-message-summary\r\nContent-Length: {}\r\n\r\n{}",
        username,
        domain,
        domain,
        cseq,
        username,
        domain,
        username,
        domain,
        subscriber_tag,
        call_id,
        cseq,
        body.len(),
        body
    )
}

fn extract_param(header: &str, name: &str) -> Option<String> {
    header.split(';').skip(1).find_map(|part| {
        let mut kv = part.trim().splitn(2, '=');
        let key = kv.next()?.trim();
        let value = kv.next()?.trim();
        (key.eq_ignore_ascii_case(name)).then(|| value.trim_matches('"').to_string())
    })
}
```

- [ ] **Step 4: Export module and run tests**

Add to `backend\src\sip\mod.rs`:

```rust
pub mod voicemail_mwi;
```

Run:

```powershell
cd backend
cargo test sip::voicemail_mwi --lib
```

Expected: PASS.

- [ ] **Step 5: Commit**

```powershell
git add -- backend\src\sip\voicemail_mwi.rs backend\src\sip\mod.rs
git commit -m "feat: add voicemail MWI helpers" -m "Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"
```

## Task 5: Admin voicemail API

**Files:**
- Create: `backend\src\api\voicemail.rs`
- Modify: `backend\src\api\mod.rs`

- [ ] **Step 1: Add API handler file**

Create `backend\src\api\voicemail.rs` with handlers following the existing `(StatusCode, String)` pattern:

```rust
use axum::{
    Json,
    body::Body,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::Response,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::AppState;
use crate::models::{
    CreateVoicemailBox, UpdateVoicemailBox, UpdateVoicemailMessage, VoicemailBox,
    VoicemailBoxSummary, VoicemailMessage, validate_box_limits, validate_voicemail_status,
};
use crate::storage::voicemail::LocalVoicemailStorage;
```

Implement:

- `list_boxes`: joins boxes to messages and returns counts.
- `create_box`: validates account exists in `sip_accounts`, applies defaults, inserts row, handles unique violation.
- `update_box`: validates limits when provided, updates row by id.
- `list_messages`: filters by optional `box_id`, `username`, and `status`.
- `download_message`: reads `storage_key` from DB and returns `audio/wav`.
- `update_message`: validates status; sets `heard_at = NOW()` for `saved` or `deleted`.
- `delete_message`: sets status `deleted`; file removal happens in retention cleanup, not inline.

Use the storage root from `state.config.server.voicemail_storage_dir`.

- [ ] **Step 2: Wire routes**

Modify imports in `backend\src\api\mod.rs`:

```rust
pub mod voicemail;
```

Add protected routes next to conference routes:

```rust
.route(
    "/api/voicemail/boxes",
    get(voicemail::list_boxes).post(voicemail::create_box),
)
.route(
    "/api/voicemail/boxes/:id",
    put(voicemail::update_box),
)
.route("/api/voicemail/messages", get(voicemail::list_messages))
.route(
    "/api/voicemail/messages/:id",
    put(voicemail::update_message).delete(voicemail::delete_message),
)
.route(
    "/api/voicemail/messages/:id/download",
    get(voicemail::download_message),
)
```

- [ ] **Step 3: Build backend**

Run:

```powershell
cd backend
cargo build
```

Expected: PASS.

- [ ] **Step 4: Commit**

```powershell
git add -- backend\src\api\voicemail.rs backend\src\api\mod.rs
git commit -m "feat: add voicemail admin API" -m "Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"
```

## Task 6: Voicemail media engine

**Files:**
- Create: `backend\src\sip\voicemail_media.rs`
- Modify: `backend\src\sip\mod.rs`

- [ ] **Step 1: Write DTMF and RTP helper tests first**

Create tests in `backend\src\sip\voicemail_media.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_dtmf_relay_signal_to_control() {
        assert_eq!(parse_dtmf_relay("Signal=#\r\nDuration=160\r\n"), Some(VoicemailDtmf::Pound));
        assert_eq!(parse_dtmf_relay("Signal=7\r\nDuration=160\r\n"), Some(VoicemailDtmf::Seven));
        assert_eq!(parse_dtmf_relay("Signal=*\r\nDuration=160\r\n"), Some(VoicemailDtmf::Star));
    }

    #[test]
    fn builds_rtp_packet_with_header() {
        let payload = [0xff, 0xfe, 0xfd];
        let packet = build_rtp_packet(0, 7, 160, 0x11223344, &payload);
        assert_eq!(packet.len(), 15);
        assert_eq!(packet[0], 0x80);
        assert_eq!(packet[1], 0);
        assert_eq!(&packet[12..], &payload);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```powershell
cd backend
cargo test sip::voicemail_media --lib
```

Expected: FAIL because the module is not exported.

- [ ] **Step 3: Implement media types and helpers**

Add:

```rust
use anyhow::{Result, anyhow};
use rand::Rng;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tracing::{debug, warn};

use super::g711::{alaw_decode, alaw_encode, ulaw_decode, ulaw_encode};
use super::voicemail_sdp::VoicemailCodec;
use crate::storage::voicemail::{LocalVoicemailStorage, pcm16_wav_bytes, read_pcm16_wav};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoicemailDtmf {
    One,
    Two,
    Seven,
    Nine,
    Pound,
    Star,
}

#[derive(Debug, Clone)]
pub struct VoicemailMedia {
    public_ip: String,
    min_port: u16,
    max_port: u16,
    sessions: Arc<Mutex<HashMap<String, MediaSession>>>,
}

#[derive(Debug, Clone)]
struct MediaSession {
    socket: Arc<UdpSocket>,
    port: u16,
    codec: VoicemailCodec,
    audio_pt: u8,
    telephone_event_pt: Option<u8>,
    peer: Arc<Mutex<Option<SocketAddr>>>,
}
```

Implement:

- `VoicemailMedia::new(public_ip, min_port, max_port)`.
- `VoicemailMedia::public_ip(&self) -> &str`.
- `VoicemailMedia::allocate(call_id, codec, audio_pt, telephone_event_pt) -> Result<u16>`.
- `VoicemailMedia::remove(call_id)`.
- `record_to_storage(call_id, mailbox, max_secs, idle_secs, storage) -> Result<(String, u32)>`.
- `play_wav(call_id, key, storage) -> Result<()>`.
- `parse_dtmf_relay(body: &str) -> Option<VoicemailDtmf>`.
- `build_rtp_packet(pt, seq, ts, ssrc, payload) -> Vec<u8>`.
- `rtp_payload_offset(packet: &[u8]) -> Option<usize>`.

For the first implementation, recording can collect PCM samples in memory up to `max_secs * 8000` samples and then write one WAV at finalization. This is bounded by default 120 seconds and avoids partial header rewriting complexity.

- [ ] **Step 4: Export module and run tests**

Add to `backend\src\sip\mod.rs`:

```rust
pub mod voicemail_media;
```

Run:

```powershell
cd backend
cargo test sip::voicemail_media --lib
```

Expected: PASS.

- [ ] **Step 5: Commit**

```powershell
git add -- backend\src\sip\voicemail_media.rs backend\src\sip\mod.rs
git commit -m "feat: add voicemail media engine" -m "Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"
```

## Task 7: Voicemail SIP endpoint for offline delivery and `*97`

**Files:**
- Create: `backend\src\sip\voicemail.rs`
- Modify: `backend\src\sip\mod.rs`

- [ ] **Step 1: Write endpoint helper tests first**

Create tests in `backend\src\sip\voicemail.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_access_extension() {
        assert!(is_voicemail_access_target("*97", "*97"));
        assert!(!is_voicemail_access_target("1001", "*97"));
    }

    #[test]
    fn adds_to_tag_when_missing() {
        let out = with_to_tag("<sip:1001@sip.air32.cn>", "vm-abc");
        assert!(out.contains("tag=vm-abc"));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```powershell
cd backend
cargo test sip::voicemail --lib
```

Expected: FAIL because module is not exported.

- [ ] **Step 3: Implement endpoint skeleton**

Create:

```rust
use anyhow::Result;
use rand::Rng;
use sqlx::MySqlPool;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use super::handler::{SipMessage, base_response, extract_uri, uri_username};
use super::proxy::CALLER_ACCOUNT_EXISTS_SQL;
use super::voicemail_media::{VoicemailDtmf, VoicemailMedia, parse_dtmf_relay};
use super::voicemail_mwi::VoicemailMwi;
use super::voicemail_sdp::{build_answer, negotiate_offer};
use crate::config::Config;
use crate::storage::voicemail::LocalVoicemailStorage;
```

Define:

```rust
#[derive(Clone)]
pub struct Voicemail {
    pool: MySqlPool,
    cfg: Config,
    media: VoicemailMedia,
    mwi: VoicemailMwi,
    active: Arc<Mutex<HashMap<String, VoicemailCall>>>,
    no_answer: Arc<Mutex<HashMap<String, tokio::task::JoinHandle<()>>>>,
}

#[derive(Debug, Clone)]
enum VoicemailMode {
    Recording { box_id: u64, mailbox: String },
    Playback { mailbox: String },
}

#[derive(Debug, Clone)]
struct VoicemailCall {
    mode: VoicemailMode,
    caller: String,
    callee: String,
}
```

Implement:

- `new(pool, cfg, media, mwi)`.
- `is_voicemail_call(call_id)`.
- `is_access_invite(msg)`.
- `lookup_enabled_box(username, domain) -> Option<MailboxSettings>`.
- `handle_access_invite(msg, src) -> Result<String>`.
- `handle_delivery_invite(msg, src, callee) -> Result<String>`.
- `handle_ack`, `handle_bye`, `handle_cancel`, `handle_info`.
- `cancel_no_answer_timer(call_id)`.
- `reconcile_on_startup`: remove expired MWI subscriptions and delete temp files under storage dir.

`handle_access_invite` and `handle_delivery_invite` both negotiate SDP, allocate media, answer `200 OK`, and insert active call state. Delivery mode starts `record_to_storage` in a spawned task after ACK or immediately after 200 OK if ACK is not needed for the first version.

- [ ] **Step 4: Implement SIP response helpers**

Add:

```rust
pub fn is_voicemail_access_target(target: &str, access_extension: &str) -> bool {
    target == access_extension
}

fn with_to_tag(to: &str, tag: &str) -> String {
    if to.to_lowercase().contains(";tag=") {
        to.to_string()
    } else {
        format!("{};tag={}", to, tag)
    }
}

fn epoch_id() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_else(|_| rand::rng().random())
}
```

Build local `200 OK` like conference:

```rust
let to_tag = format!("vm-{:x}", epoch_id());
let to_with_tag = with_to_tag(msg.to_header().unwrap_or(""), &to_tag);
let contact = format!(
    "<sip:{}@{}:{}>",
    self.cfg.server.voicemail_access_extension,
    self.cfg.server.public_ip,
    self.cfg.server.sip_port
);
let response = base_response_with_to(msg, 200, "OK", &to_with_tag)
    .header("Contact", &contact)
    .header("Content-Type", "application/sdp")
    .header("Allow", "INVITE, ACK, CANCEL, BYE, INFO")
    .body(&answer)
    .build();
```

Copy the `base_response_with_to` pattern from `conference.rs` into `voicemail.rs` so local To-tags are explicit.

- [ ] **Step 5: Export module and run tests**

Add to `backend\src\sip\mod.rs`:

```rust
pub mod voicemail;
```

Run:

```powershell
cd backend
cargo test sip::voicemail --lib
```

Expected: PASS.

- [ ] **Step 6: Commit**

```powershell
git add -- backend\src\sip\voicemail.rs backend\src\sip\mod.rs
git commit -m "feat: add voicemail SIP endpoint" -m "Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"
```

## Task 8: Handler and proxy routing

**Files:**
- Modify: `backend\src\sip\handler.rs`
- Modify: `backend\src\sip\proxy.rs`

- [ ] **Step 1: Add voicemail to `SipHandler` construction**

In `backend\src\sip\handler.rs`, import:

```rust
use super::voicemail::Voicemail;
use super::voicemail_media::VoicemailMedia;
use super::voicemail_mwi::VoicemailMwi;
```

Add field:

```rust
voicemail: Voicemail,
```

In `with_socket`, construct after `presence`:

```rust
let voicemail_media = VoicemailMedia::new(
    cfg.server.public_ip.clone(),
    cfg.server.voicemail_rtp_port_min,
    cfg.server.voicemail_rtp_port_max,
);
let voicemail_mwi = VoicemailMwi::new(pool.clone(), cfg.clone(), socket.clone());
let voicemail = Voicemail::new(pool.clone(), cfg.clone(), voicemail_media, voicemail_mwi);
```

Add accessor:

```rust
pub fn voicemail(&self) -> &Voicemail {
    &self.voicemail
}
```

- [ ] **Step 2: Route `*97`, active voicemail dialogs, and message-summary SUBSCRIBE**

In `process_sip_msg`, before conference routing, add:

```rust
let call_id_str = msg.call_id().unwrap_or("").to_string();
let is_vm = !call_id_str.is_empty() && self.voicemail.is_voicemail_call(&call_id_str).await;

if method == "SUBSCRIBE" {
    let event = msg.header("event").unwrap_or("").to_lowercase();
    if event.split(';').next().unwrap_or("").trim() == "message-summary" {
        let resp = self.voicemail.mwi().handle_subscribe(&msg, src).await;
        return finalize_response(&msg, resp, &method);
    }
}

if method == "INVITE" && self.voicemail.is_access_invite(&msg) {
    let resp = self.voicemail.handle_access_invite(&msg, src).await;
    return finalize_response(&msg, resp, &method);
}

if is_vm {
    match method.as_str() {
        "ACK" => {
            self.voicemail.handle_ack(&msg).await;
            return Ok(None);
        }
        "BYE" => {
            let resp = self.voicemail.handle_bye(&msg).await;
            return finalize_response(&msg, resp, &method);
        }
        "CANCEL" => {
            let resp = self.voicemail.handle_cancel(&msg).await;
            return finalize_response(&msg, resp, &method);
        }
        "INFO" => {
            let resp = self.voicemail.handle_info(&msg).await;
            return finalize_response(&msg, resp, &method);
        }
        _ => {}
    }
}
```

Expose `mwi()` from `Voicemail`:

```rust
pub fn mwi(&self) -> &VoicemailMwi {
    &self.mwi
}
```

- [ ] **Step 3: Pass voicemail into `Proxy`**

Modify `Proxy::new` signature to accept `voicemail: Voicemail` and store it:

```rust
voicemail: Voicemail,
```

Update the `Proxy::new(...)` call in `SipHandler::with_socket` to pass `voicemail.clone()`.

- [ ] **Step 4: Route unregistered callees to voicemail**

In `Proxy::handle_invite`, replace the unregistered branch:

```rust
None => {
    warn!("INVITE to unregistered user: {}", callee);
    return Ok(base_response(msg, 404, "Not Found").build());
}
```

with:

```rust
None => {
    warn!("INVITE to unregistered user: {}", callee);
    if self.voicemail.lookup_enabled_box(&callee, &domain).await.is_some() {
        return self.voicemail.handle_delivery_invite(msg, src, &callee).await;
    }
    return Ok(base_response(msg, 404, "Not Found").build());
}
```

- [ ] **Step 5: Start and cancel no-answer timers**

After forwarding an INVITE successfully, call:

```rust
self.voicemail
    .start_no_answer_timer(
        msg.clone(),
        src,
        target_addr,
        callee.clone(),
        self.transport_registry.clone(),
        self.socket.clone(),
    )
    .await;
```

In `SipHandler::relay_response`, when any final response arrives for the call, add before pending dialog cleanup:

```rust
if msg.status_code.is_some_and(|c| c >= 200) {
    self.voicemail.cancel_no_answer_timer(&call_id).await;
}
```

In `Proxy::handle_ack`, add:

```rust
self.voicemail.cancel_no_answer_timer(&call_id).await;
```

- [ ] **Step 6: Build and run focused backend tests**

```powershell
cd backend
cargo build
cargo test sip::voicemail --lib
```

Expected: PASS.

- [ ] **Step 7: Commit**

```powershell
git add -- backend\src\sip\handler.rs backend\src\sip\proxy.rs
git commit -m "feat: route calls to voicemail" -m "Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"
```

## Task 9: Server startup cleanup and deployment wiring

**Files:**
- Modify: `backend\src\sip\server.rs`
- Modify: `docker-compose.yml`

- [ ] **Step 1: Add startup reconciliation**

In `backend\src\sip\server.rs`, after conference reconciliation:

```rust
if let Err(e) = handler.voicemail().reconcile_on_startup().await {
    warn!("Voicemail startup reconciliation failed: {}", e);
}
```

- [ ] **Step 2: Add Docker environment and UDP port mapping**

In `docker-compose.yml`, add backend environment:

```yaml
SIP3__SERVER__VOICEMAIL_ACCESS_EXTENSION: "*97"
SIP3__SERVER__VOICEMAIL_RTP_PORT_MIN: 10200
SIP3__SERVER__VOICEMAIL_RTP_PORT_MAX: 10299
SIP3__SERVER__VOICEMAIL_STORAGE_DIR: /app/voicemail
SIP3__SERVER__VOICEMAIL_PROMPT_DIR: /app/voicemail/prompts
```

Add ports:

```yaml
- "10200-10299:10200-10299/udp"
```

Add volume:

```yaml
- ./voicemail:/app/voicemail
```

- [ ] **Step 3: Build backend**

```powershell
cd backend
cargo build
```

Expected: PASS.

- [ ] **Step 4: Commit**

```powershell
git add -- backend\src\sip\server.rs docker-compose.yml
git commit -m "feat: wire voicemail runtime configuration" -m "Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"
```

## Task 10: Frontend voicemail admin UI

**Files:**
- Create: `frontend\src\views\Voicemail.vue`
- Modify: `frontend\src\router\index.js`
- Modify: `frontend\src\components\SideNav.vue`

- [ ] **Step 1: Create `Voicemail.vue`**

Use direct `api` calls, following `Conferences.vue` style. Required state:

```js
const boxes = ref([])
const messages = ref([])
const loadingBoxes = ref(false)
const loadingMessages = ref(false)
const selectedBox = ref(null)
const drawerVisible = ref(false)
const form = ref({ enabled: 1, no_answer_secs: 25, max_message_secs: 120, max_messages: 100 })
```

Required methods:

```js
const fetchBoxes = async () => {
  loadingBoxes.value = true
  try {
    const res = await api.get('/voicemail/boxes')
    boxes.value = res.data.data || []
  } finally {
    loadingBoxes.value = false
  }
}

const fetchMessages = async (box) => {
  selectedBox.value = box
  loadingMessages.value = true
  try {
    const res = await api.get('/voicemail/messages', { params: { box_id: box.id } })
    messages.value = res.data.data || []
  } finally {
    loadingMessages.value = false
  }
}

const downloadMessage = (row) => {
  window.open(`/api/voicemail/messages/${row.id}/download`, '_blank')
}

const updateMessageStatus = async (row, status) => {
  await api.put(`/voicemail/messages/${row.id}`, { status })
  await fetchMessages(selectedBox.value)
  await fetchBoxes()
}
```

The template must include:

- mailbox table with username/domain/enabled/new_count/saved_count/no_answer_secs/max_message_secs/max_messages;
- message table with caller/duration/status/created_at;
- actions for play/download/save/delete;
- form validation for positive numeric limits.

- [ ] **Step 2: Add route**

Modify `frontend\src\router\index.js`:

```js
import Voicemail from '../views/Voicemail.vue'
```

Add route:

```js
{ path: '/voicemail', component: Voicemail },
```

- [ ] **Step 3: Add nav item**

Modify `frontend\src\components\SideNav.vue` imports:

```js
import { MessageBox } from '@element-plus/icons-vue'
```

Add:

```js
{ path: '/voicemail', label: '语音信箱', icon: MessageBox },
```

- [ ] **Step 4: Run frontend build**

```powershell
cd frontend
npm run build
```

Expected: PASS.

- [ ] **Step 5: Commit**

```powershell
git add -- frontend\src\views\Voicemail.vue frontend\src\router\index.js frontend\src\components\SideNav.vue
git commit -m "feat: add voicemail admin UI" -m "Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"
```

## Task 11: Documentation and full verification

**Files:**
- Modify: `README.md`
- Modify: `docs\deployment.md`

- [ ] **Step 1: Document voicemail behavior**

Add documentation covering:

- `*97` mailbox access.
- Offline and 25-second no-answer delivery.
- G.711 PCMU/PCMA RTP/AVP only.
- MWI via `SUBSCRIBE Event: message-summary`.
- WAV storage directory and prompt directory.
- RTP range `10200-10299`.
- DTMF menu: `1`, `2/#`, `7`, `9`, `*`.
- MVP exclusions: PIN, busy-to-voicemail, email notifications, SRTP, Opus, browser/WebRTC voicemail.

- [ ] **Step 2: Run backend formatting**

```powershell
cd backend
cargo fmt
```

Expected: command exits 0.

- [ ] **Step 3: Run backend tests**

```powershell
cd backend
cargo test
```

Expected: all tests pass.

- [ ] **Step 4: Run backend clippy**

```powershell
cd backend
cargo clippy -- -D warnings
```

Expected: clippy exits 0.

- [ ] **Step 5: Run frontend build**

```powershell
cd frontend
npm run build
```

Expected: production build completes.

- [ ] **Step 6: Commit docs and verification fixes**

```powershell
git add -- README.md docs\deployment.md
git commit -m "docs: document voicemail feature" -m "Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"
```

If formatting changed Rust files during verification, include those exact files in the same commit only if they belong to the voicemail tasks.

## Manual Linphone smoke test checklist

- [ ] Register `1001` and `1002` in Linphone.
- [ ] Enable voicemail box for `1002`.
- [ ] Unregister `1002`; call `1002` from `1001`; confirm voicemail answers and records.
- [ ] Re-register `1002`; call `1002` from `1001`; do not answer; confirm voicemail answers after 25 seconds and the callee stops ringing.
- [ ] Dial `*97` from `1002`; confirm prompts play and new messages are audible.
- [ ] During playback, press `1`, `2`, `#`, `7`, `9`, and `*`; confirm each action matches the menu.
- [ ] Enable MWI subscription in Linphone; confirm new voicemail toggles waiting state and saved/deleted messages update counts.

## Plan self-review

- Spec coverage: offline delivery, no-answer delivery, `*97`, local WAV storage abstraction, MWI, admin UI, config, docs, and tests are covered by Tasks 1-11.
- Red-flag scan: no incomplete markers remain in this plan.
- Type consistency: voicemail modules consistently use `Voicemail`, `VoicemailMedia`, `VoicemailMwi`, `VoicemailCodec`, `VoicemailNegotiation`, and `LocalVoicemailStorage`.
