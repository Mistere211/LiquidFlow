//! 2FA-Codes (TOTP, RFC 6238): Secret oder otpauth://-Link -> 6-stelliger Code.
use hmac::{Hmac, Mac};
use sha1::Sha1;
use sha2::{Sha256, Sha512};

pub struct Totp { pub secret: Vec<u8>, pub digits: u32, pub period: u64, pub algo: String }

pub fn base32_decode(s: &str) -> Option<Vec<u8>> {
    let (mut bits, mut n, mut out) = (0u32, 0u32, vec![]);
    for c in s.chars().filter(|c| !c.is_whitespace() && *c != '-' && *c != '=') {
        let u = c.to_ascii_uppercase();
        let v = match u { 'A'..='Z' => u as u32 - 'A' as u32, '2'..='7' => u as u32 - '2' as u32 + 26, _ => return None };
        bits = (bits << 5) | v;
        n += 5;
        if n >= 8 { n -= 8; out.push((bits >> n) as u8); bits &= (1 << n) - 1; }
    }
    (!out.is_empty()).then_some(out)
}

/// Nimmt ein Base32-Secret oder einen otpauth://totp/…?secret=…-Link.
pub fn parse(input: &str) -> Option<Totp> {
    let input = input.trim();
    if input.starts_with("otpauth://") {
        let u = url::Url::parse(input).ok()?;
        let q: std::collections::HashMap<_, _> = u.query_pairs().into_owned().collect();
        return Some(Totp { secret: base32_decode(q.get("secret")?)?,
            digits: q.get("digits").and_then(|d| d.parse().ok()).filter(|d| (6..=8).contains(d)).unwrap_or(6),
            period: q.get("period").and_then(|d| d.parse().ok()).filter(|p| *p > 0).unwrap_or(30),
            algo: q.get("algorithm").map(|a| a.to_uppercase()).unwrap_or_else(|| "SHA1".into()) });
    }
    Some(Totp { secret: base32_decode(input)?, digits: 6, period: 30, algo: "SHA1".into() })
}

fn mac(algo: &str, key: &[u8], msg: &[u8]) -> Vec<u8> {
    match algo {
        "SHA256" => { let mut m = Hmac::<Sha256>::new_from_slice(key).unwrap(); m.update(msg); m.finalize().into_bytes().to_vec() }
        "SHA512" => { let mut m = Hmac::<Sha512>::new_from_slice(key).unwrap(); m.update(msg); m.finalize().into_bytes().to_vec() }
        _ => { let mut m = Hmac::<Sha1>::new_from_slice(key).unwrap(); m.update(msg); m.finalize().into_bytes().to_vec() }
    }
}

pub fn code(t: &Totp, unix: u64) -> String {
    let h = mac(&t.algo, &t.secret, &(unix / t.period).to_be_bytes());
    let o = (h[h.len() - 1] & 0xf) as usize;
    let bin = ((h[o] as u32 & 0x7f) << 24) | ((h[o + 1] as u32) << 16) | ((h[o + 2] as u32) << 8) | h[o + 3] as u32;
    format!("{:0width$}", bin % 10u32.pow(t.digits), width = t.digits as usize)
}

pub fn remaining(t: &Totp, unix: u64) -> u64 { t.period - unix % t.period }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rfc6238_vectors() {
        let t = |s: &[u8], a: &str| Totp { secret: s.to_vec(), digits: 8, period: 30, algo: a.into() };
        assert_eq!(code(&t(b"12345678901234567890", "SHA1"), 59), "94287082");
        assert_eq!(code(&t(b"12345678901234567890123456789012", "SHA256"), 59), "46119246");
        assert_eq!(code(&t(b"1234567890123456789012345678901234567890123456789012345678901234", "SHA512"), 59), "90693936");
    }
    #[test]
    fn base32_and_uri() {
        assert_eq!(base32_decode("GEZDGNBVGY3TQOJQ").unwrap(), b"1234567890");
        let t = parse("otpauth://totp/Test:me?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&digits=8&issuer=Test").unwrap();
        assert_eq!(t.digits, 8);
        assert_eq!(code(&t, 59), "94287082");
        assert!(parse("nicht base32!").is_none());
    }
}
