//! Conference SDP offer parsing and answer generation.
//!
//! MVP supports only RTP/AVP G.711 PCMU (PT 0) and PCMA (PT 8). SAVP/SAVPF
//! offers without an RTP/AVP audio section are rejected with 488. Telephone-event
//! (DTMF, RFC 4733) payload types are preserved when present so callers can send
//! `*6` mute/unmute via RFC 2833.

use anyhow::{Result, anyhow};

/// Audio codecs the conference mixer can encode/decode.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ConferenceCodec {
    Pcmu,
    Pcma,
}

impl ConferenceCodec {
    pub fn rtpmap_name(self) -> &'static str {
        match self {
            ConferenceCodec::Pcmu => "PCMU",
            ConferenceCodec::Pcma => "PCMA",
        }
    }

    /// Static payload type from RFC 3551.
    pub fn static_pt(self) -> u8 {
        match self {
            ConferenceCodec::Pcmu => 0,
            ConferenceCodec::Pcma => 8,
        }
    }
}

/// Outcome of negotiating an offer for one audio media section.
#[derive(Debug, Clone)]
pub struct ConferenceNegotiation {
    pub codec: ConferenceCodec,
    /// Negotiated audio payload type echoed back in the answer.
    pub audio_pt: u8,
    /// Optional RFC 4733 telephone-event payload type and clock rate.
    pub telephone_event_pt: Option<u8>,
}

/// Parse the first audio `m=` section of an SDP offer and pick a supported codec.
///
/// Returns Err if there is no `m=audio` line using the RTP/AVP profile that
/// advertises PCMU or PCMA.
pub fn negotiate_offer(sdp: &str) -> Result<ConferenceNegotiation> {
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

    let mut chosen: Option<(ConferenceCodec, u8)> = None;
    let mut telephone_event_pt: Option<u8> = None;

    for pt in &audio_pts {
        match resolve(*pt) {
            Some(("PCMU", 8000)) if chosen.is_none() => {
                chosen = Some((ConferenceCodec::Pcmu, *pt));
            }
            Some(("PCMA", 8000)) if chosen.is_none() => {
                chosen = Some((ConferenceCodec::Pcma, *pt));
            }
            Some(("telephone-event", 8000)) if telephone_event_pt.is_none() => {
                telephone_event_pt = Some(*pt);
            }
            _ => {}
        }
    }

    let (codec, audio_pt) =
        chosen.ok_or_else(|| anyhow!("no PCMU or PCMA payload type offered for audio"))?;

    Ok(ConferenceNegotiation {
        codec,
        audio_pt,
        telephone_event_pt,
    })
}

/// Build a Linphone-compatible SDP answer for the conference endpoint.
pub fn build_answer(
    public_ip: &str,
    relay_port: u16,
    negotiation: &ConferenceNegotiation,
    session_id: u64,
) -> String {
    let mut pts = format!("{}", negotiation.audio_pt);
    if let Some(te) = negotiation.telephone_event_pt {
        pts.push(' ');
        pts.push_str(&te.to_string());
    }

    let mut sdp = String::new();
    sdp.push_str("v=0\r\n");
    sdp.push_str(&format!(
        "o=sip3 {sid} {sid} IN IP4 {ip}\r\n",
        sid = session_id,
        ip = public_ip
    ));
    sdp.push_str("s=SIP3 Conference\r\n");
    sdp.push_str(&format!("c=IN IP4 {}\r\n", public_ip));
    sdp.push_str("t=0 0\r\n");
    sdp.push_str(&format!("m=audio {} RTP/AVP {}\r\n", relay_port, pts));
    sdp.push_str(&format!(
        "a=rtpmap:{} {}/8000\r\n",
        negotiation.audio_pt,
        negotiation.codec.rtpmap_name()
    ));
    if let Some(te) = negotiation.telephone_event_pt {
        sdp.push_str(&format!("a=rtpmap:{} telephone-event/8000\r\n", te));
        sdp.push_str(&format!("a=fmtp:{} 0-15\r\n", te));
    }
    sdp.push_str("a=ptime:20\r\n");
    sdp.push_str("a=sendrecv\r\n");
    sdp
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINPHONE_OFFER: &str = "v=0\r\n\
o=alice 1 1 IN IP4 192.168.1.10\r\n\
s=Talk\r\n\
c=IN IP4 192.168.1.10\r\n\
t=0 0\r\n\
m=audio 7078 RTP/AVP 96 8 0 101\r\n\
a=rtpmap:96 opus/48000/2\r\n\
a=rtpmap:8 PCMA/8000\r\n\
a=rtpmap:0 PCMU/8000\r\n\
a=rtpmap:101 telephone-event/8000\r\n\
a=fmtp:101 0-15\r\n";

    #[test]
    fn negotiates_pcma_from_linphone_offer() {
        let n = negotiate_offer(LINPHONE_OFFER).expect("must negotiate");
        assert_eq!(n.codec, ConferenceCodec::Pcma);
        assert_eq!(n.audio_pt, 8);
        assert_eq!(n.telephone_event_pt, Some(101));
    }

    #[test]
    fn rejects_savp_only_offer() {
        let sdp = "v=0\r\nm=audio 5000 RTP/SAVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
        assert!(negotiate_offer(sdp).is_err());
    }

    #[test]
    fn rejects_offer_without_g711() {
        let sdp = "v=0\r\nm=audio 5000 RTP/AVP 96\r\na=rtpmap:96 opus/48000/2\r\n";
        assert!(negotiate_offer(sdp).is_err());
    }

    #[test]
    fn rejects_offer_without_audio() {
        let sdp = "v=0\r\nm=video 5000 RTP/AVP 96\r\n";
        assert!(negotiate_offer(sdp).is_err());
    }

    #[test]
    fn picks_pcmu_when_pcma_absent() {
        let sdp = "v=0\r\nm=audio 5000 RTP/AVP 0 101\r\na=rtpmap:0 PCMU/8000\r\na=rtpmap:101 telephone-event/8000\r\n";
        let n = negotiate_offer(sdp).unwrap();
        assert_eq!(n.codec, ConferenceCodec::Pcmu);
        assert_eq!(n.audio_pt, 0);
        assert_eq!(n.telephone_event_pt, Some(101));
    }

    #[test]
    fn answer_includes_codec_and_dtmf() {
        let n = ConferenceNegotiation {
            codec: ConferenceCodec::Pcmu,
            audio_pt: 0,
            telephone_event_pt: Some(101),
        };
        let sdp = build_answer("203.0.113.5", 10100, &n, 42);
        assert!(sdp.contains("m=audio 10100 RTP/AVP 0 101"));
        assert!(sdp.contains("a=rtpmap:0 PCMU/8000"));
        assert!(sdp.contains("a=rtpmap:101 telephone-event/8000"));
        assert!(sdp.contains("a=fmtp:101 0-15"));
        assert!(sdp.contains("a=ptime:20"));
        assert!(sdp.contains("a=sendrecv"));
        assert!(sdp.contains("c=IN IP4 203.0.113.5"));
    }

    #[test]
    fn answer_without_dtmf_lists_only_audio_pt() {
        let n = ConferenceNegotiation {
            codec: ConferenceCodec::Pcma,
            audio_pt: 8,
            telephone_event_pt: None,
        };
        let sdp = build_answer("198.51.100.7", 10120, &n, 7);
        assert!(sdp.contains("m=audio 10120 RTP/AVP 8\r\n"));
        assert!(!sdp.contains("telephone-event"));
    }
}
