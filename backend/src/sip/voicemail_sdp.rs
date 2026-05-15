//! Voicemail SDP offer parsing and answer generation.
//!
//! MVP supports only RTP/AVP G.711 PCMU (PT 0) and PCMA (PT 8). SAVP/SAVPF
//! offers without an RTP/AVP audio section are rejected with 488. Telephone-event
//! (DTMF, RFC 4733) payload types are preserved when present for DTMF-based
//! voicemail menu navigation.

use anyhow::{Result, anyhow};

/// Audio codecs the voicemail system can encode/decode.
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

/// Outcome of negotiating an offer for one audio media section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoicemailNegotiation {
    pub codec: VoicemailCodec,
    /// Negotiated audio payload type echoed back in the answer.
    pub audio_pt: u8,
    /// Optional RFC 4733 telephone-event payload type.
    pub telephone_event_pt: Option<u8>,
}

/// Parse the first audio `m=` section of an SDP offer and pick a supported codec.
///
/// Prefers PCMU when both are offered. Returns Err if there is no `m=audio` line
/// using the RTP/AVP profile that advertises PCMU or PCMA.
pub fn negotiate_offer(sdp: &str) -> Result<VoicemailNegotiation> {
    let mut in_audio = false;
    let mut audio_pts: Vec<u8> = Vec::new();
    let mut profile_ok = false;
    let mut rtpmaps: Vec<(u8, String, u32)> = Vec::new(); // (pt, name_upper, clock)

    for line in sdp.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("m=") {
            // New media section starts; only consider the first audio section.
            if in_audio {
                break;
            }
            let mut parts = rest.split_whitespace();
            let kind = parts.next().unwrap_or("");
            if kind != "audio" {
                continue;
            }
            in_audio = true;
            let _port = parts.next();
            let profile = parts.next().unwrap_or("");
            profile_ok = profile == "RTP/AVP";
            for pt in parts {
                if let Ok(v) = pt.parse::<u8>() {
                    audio_pts.push(v);
                }
            }
            continue;
        }

        if !in_audio {
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("a=rtpmap:") {
            // Format: "<pt> <name>/<clock>[/<channels>]"
            let mut sp = rest.split_whitespace();
            let pt_str = sp.next().unwrap_or("");
            let mapping = sp.next().unwrap_or("");
            let pt = match pt_str.parse::<u8>() {
                Ok(v) => v,
                Err(_) => continue,
            };
            let mut mp = mapping.split('/');
            let name = mp.next().unwrap_or("").to_ascii_uppercase();
            let clock: u32 = mp.next().and_then(|c| c.parse().ok()).unwrap_or(0);
            rtpmaps.push((pt, name, clock));
        }
    }

    if !in_audio {
        return Err(anyhow!("no m=audio section in offer"));
    }
    if !profile_ok {
        return Err(anyhow!(
            "audio profile is not RTP/AVP (encrypted-only offer)"
        ));
    }

    // Resolve each advertised PT to a name (using static defaults for PT 0/8).
    let resolve = |pt: u8| -> Option<(&'static str, u32)> {
        if let Some((_, name, clock)) = rtpmaps.iter().find(|(p, _, _)| *p == pt) {
            // Map upper-case dynamic name back to a known codec string when possible.
            let s: &str = match name.as_str() {
                "PCMU" => "PCMU",
                "PCMA" => "PCMA",
                "TELEPHONE-EVENT" => "telephone-event",
                _ => return None,
            };
            return Some((s, *clock));
        }
        // Static fallback per RFC 3551.
        match pt {
            0 => Some(("PCMU", 8000)),
            8 => Some(("PCMA", 8000)),
            _ => None,
        }
    };

    let mut pcmu_candidate: Option<u8> = None;
    let mut pcma_candidate: Option<u8> = None;
    let mut telephone_event_pt: Option<u8> = None;

    // Single pass: record PCMU, PCMA fallback, and telephone-event independently
    for pt in &audio_pts {
        match resolve(*pt) {
            Some(("PCMU", 8000)) => {
                pcmu_candidate = Some(*pt);
            }
            Some(("PCMA", 8000)) if pcma_candidate.is_none() => {
                pcma_candidate = Some(*pt);
            }
            Some(("telephone-event", 8000)) if telephone_event_pt.is_none() => {
                telephone_event_pt = Some(*pt);
            }
            _ => {}
        }
    }

    // Prefer PCMU, fallback to PCMA
    let (codec, audio_pt) = if let Some(pt) = pcmu_candidate {
        (VoicemailCodec::Pcmu, pt)
    } else if let Some(pt) = pcma_candidate {
        (VoicemailCodec::Pcma, pt)
    } else {
        return Err(anyhow!("no PCMU or PCMA payload type offered for audio"));
    };

    Ok(VoicemailNegotiation {
        codec,
        audio_pt,
        telephone_event_pt,
    })
}

/// Build a Linphone-compatible SDP answer for the voicemail endpoint.
pub fn build_answer(
    public_ip: &str,
    relay_port: u16,
    negotiation: &VoicemailNegotiation,
    session_id: u64,
) -> String {
    let mut pts = negotiation.audio_pt.to_string();
    if let Some(dtmf_pt) = negotiation.telephone_event_pt {
        pts.push(' ');
        pts.push_str(&dtmf_pt.to_string());
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
    fn answer_includes_codec_and_dtmf() {
        let n = VoicemailNegotiation {
            codec: VoicemailCodec::Pcma,
            audio_pt: 8,
            telephone_event_pt: Some(101),
        };
        let answer = build_answer("203.0.113.10", 10200, &n, 1234);
        assert!(answer.contains("m=audio 10200 RTP/AVP 8 101"));
        assert!(answer.contains("a=rtpmap:8 PCMA/8000"));
        assert!(answer.contains("a=rtpmap:101 telephone-event/8000"));
        assert!(answer.contains("a=fmtp:101 0-15"));
        assert!(answer.contains("s=SIP3 Voicemail"));
    }

    #[test]
    fn answer_without_dtmf_omits_telephone_event() {
        let n = VoicemailNegotiation {
            codec: VoicemailCodec::Pcmu,
            audio_pt: 0,
            telephone_event_pt: None,
        };
        let answer = build_answer("203.0.113.10", 10200, &n, 1234);
        assert!(answer.contains("m=audio 10200 RTP/AVP 0\r\n"));
        assert!(answer.contains("a=rtpmap:0 PCMU/8000"));
        assert!(!answer.contains("telephone-event"));
    }

    #[test]
    fn captures_dtmf_when_pcma_listed_before_pcmu() {
        let offer = "v=0\r\nm=audio 5000 RTP/AVP 8 101 0\r\na=rtpmap:8 PCMA/8000\r\na=rtpmap:101 telephone-event/8000\r\na=rtpmap:0 PCMU/8000\r\n";
        let n = negotiate_offer(offer).expect("negotiate");
        assert_eq!(n.codec, VoicemailCodec::Pcmu);
        assert_eq!(n.audio_pt, 0);
        assert_eq!(n.telephone_event_pt, Some(101));
    }

    #[test]
    fn picks_pcmu_over_pcma_when_pcma_first() {
        let offer = "v=0\r\nm=audio 5000 RTP/AVP 8 0\r\na=rtpmap:8 PCMA/8000\r\na=rtpmap:0 PCMU/8000\r\n";
        let n = negotiate_offer(offer).expect("negotiate");
        assert_eq!(n.codec, VoicemailCodec::Pcmu);
        assert_eq!(n.audio_pt, 0);
        assert_eq!(n.telephone_event_pt, None);
    }

    #[test]
    fn rejects_offer_without_g711() {
        let offer = "v=0\r\nm=audio 5000 RTP/AVP 96\r\na=rtpmap:96 opus/48000/2\r\n";
        assert!(negotiate_offer(offer).is_err());
    }

    #[test]
    fn rejects_offer_without_audio() {
        let offer = "v=0\r\nm=video 5000 RTP/AVP 96\r\n";
        assert!(negotiate_offer(offer).is_err());
    }
}
