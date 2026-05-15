//! ITU-T G.711 PCMU (μ-law) and PCMA (A-law) codec helpers.
//!
//! These are pure transcoding functions used by the conference mixer.
//! No external crate needed; algorithm follows the standard reference.

const BIAS: i16 = 0x84;
const CLIP: i16 = 32635;

/// Encode a single 16-bit linear PCM sample to G.711 μ-law (PCMU).
pub fn linear_to_ulaw(mut pcm: i16) -> u8 {
    let sign: u8 = if pcm < 0 {
        pcm = -pcm.saturating_add(0); // careful with i16::MIN
        if pcm < 0 {
            pcm = i16::MAX;
        }
        0x80
    } else {
        0
    };
    if pcm > CLIP {
        pcm = CLIP;
    }
    pcm = pcm.saturating_add(BIAS);

    // Find segment (exponent): position of the highest set bit above bit 7.
    let mut exponent: u8 = 7;
    let mut mask: i16 = 0x4000;
    while exponent > 0 && (pcm & mask) == 0 {
        exponent -= 1;
        mask >>= 1;
    }
    let mantissa = ((pcm >> (exponent + 3)) & 0x0F) as u8;
    !(sign | (exponent << 4) | mantissa)
}

/// Decode a single G.711 μ-law byte to 16-bit linear PCM.
pub fn ulaw_to_linear(ulaw: u8) -> i16 {
    let u = !ulaw;
    let sign = u & 0x80;
    let exponent = (u >> 4) & 0x07;
    let mantissa = u & 0x0F;
    let magnitude = ((mantissa as i16) << 3 | 0x84) << exponent;
    let sample = magnitude - BIAS;
    if sign != 0 { -sample } else { sample }
}

/// Encode a single 16-bit linear PCM sample to G.711 A-law (PCMA).
pub fn linear_to_alaw(mut pcm: i16) -> u8 {
    let sign: u8 = if pcm < 0 {
        pcm = pcm.saturating_neg();
        if pcm < 0 {
            pcm = i16::MAX;
        }
        0x00
    } else {
        0x80
    };
    if pcm > CLIP {
        pcm = CLIP;
    }

    let alaw = if pcm < 256 {
        ((pcm >> 4) & 0x0F) as u8
    } else {
        let mut exponent: u8 = 7;
        let mut mask: i16 = 0x4000;
        while exponent > 0 && (pcm & mask) == 0 {
            exponent -= 1;
            mask >>= 1;
        }
        let mantissa = ((pcm >> (exponent + 3)) & 0x0F) as u8;
        (exponent << 4) | mantissa
    };
    (alaw | sign) ^ 0x55
}

/// Decode a single G.711 A-law byte to 16-bit linear PCM.
pub fn alaw_to_linear(alaw: u8) -> i16 {
    let a = alaw ^ 0x55;
    let sign = a & 0x80;
    let exponent = (a >> 4) & 0x07;
    let mantissa = (a & 0x0F) as i16;
    let magnitude = if exponent == 0 {
        (mantissa << 4) | 0x08
    } else {
        ((mantissa << 4) | 0x108) << (exponent - 1)
    };
    if sign != 0 { magnitude } else { -magnitude }
}

/// Encode a buffer of linear PCM samples to μ-law bytes.
pub fn encode_ulaw(pcm: &[i16]) -> Vec<u8> {
    pcm.iter().map(|s| linear_to_ulaw(*s)).collect()
}

/// Encode a buffer of linear PCM samples to A-law bytes.
pub fn encode_alaw(pcm: &[i16]) -> Vec<u8> {
    pcm.iter().map(|s| linear_to_alaw(*s)).collect()
}

/// Decode μ-law bytes to linear PCM samples.
pub fn decode_ulaw(payload: &[u8]) -> Vec<i16> {
    payload.iter().map(|b| ulaw_to_linear(*b)).collect()
}

/// Decode A-law bytes to linear PCM samples.
pub fn decode_alaw(payload: &[u8]) -> Vec<i16> {
    payload.iter().map(|b| alaw_to_linear(*b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ulaw_zero_round_trip() {
        let encoded = linear_to_ulaw(0);
        let decoded = ulaw_to_linear(encoded);
        assert!(decoded.abs() <= 8, "zero round-trip drift: {}", decoded);
    }

    #[test]
    fn alaw_zero_round_trip() {
        let encoded = linear_to_alaw(0);
        let decoded = alaw_to_linear(encoded);
        assert!(decoded.abs() <= 16, "zero round-trip drift: {}", decoded);
    }

    #[test]
    fn ulaw_round_trip_within_quantization() {
        // G.711 uses a logarithmic 8-bit quantization; check tolerance grows with magnitude.
        for &sample in &[100i16, 500, 1000, 5000, 10000, 20000, -100, -500, -10000] {
            let encoded = linear_to_ulaw(sample);
            let decoded = ulaw_to_linear(encoded);
            // Relative error budget: 12.5% of magnitude + small fixed offset.
            let tolerance = (sample.unsigned_abs() as i32 / 8).max(64);
            let diff = (decoded as i32 - sample as i32).abs();
            assert!(
                diff <= tolerance,
                "sample={sample} decoded={decoded} diff={diff} tol={tolerance}"
            );
        }
    }

    #[test]
    fn alaw_round_trip_within_quantization() {
        for &sample in &[100i16, 500, 1000, 5000, 10000, 20000, -100, -500, -10000] {
            let encoded = linear_to_alaw(sample);
            let decoded = alaw_to_linear(encoded);
            let tolerance = (sample.unsigned_abs() as i32 / 8).max(64);
            let diff = (decoded as i32 - sample as i32).abs();
            assert!(
                diff <= tolerance,
                "sample={sample} decoded={decoded} diff={diff} tol={tolerance}"
            );
        }
    }

    #[test]
    fn buffer_helpers_match_lengths() {
        let pcm: Vec<i16> = (0..160).map(|i| (i * 100) as i16).collect();
        assert_eq!(encode_ulaw(&pcm).len(), pcm.len());
        assert_eq!(encode_alaw(&pcm).len(), pcm.len());
        assert_eq!(decode_ulaw(&encode_ulaw(&pcm)).len(), pcm.len());
        assert_eq!(decode_alaw(&encode_alaw(&pcm)).len(), pcm.len());
    }

    #[test]
    fn ulaw_silence_is_silence() {
        // Standard μ-law silence byte is 0xFF (encoded zero).
        assert_eq!(linear_to_ulaw(0), 0xFF);
    }

    #[test]
    fn alaw_silence_is_silence() {
        // Standard A-law silence byte is 0xD5 (encoded zero with sign bit set).
        assert_eq!(linear_to_alaw(0), 0xD5);
    }
}
