//! A tap on an NFC tag, checked (attendance certificates, DECISIONS §150).
//!
//! NTAG 424 DNA tags in "Secure Unique NFC" (SUN) mode write a fresh URL on
//! every read: the tag's UID and a read counter, encrypted, and a CMAC over
//! them with a key derived per tap. A copied URL is therefore worth one tap —
//! the counter it carries is already used — and a URL nobody tapped cannot be
//! made without the tag's keys.
//!
//! The layout is NXP AN12196 ("NTAG 424 DNA and NTAG 424 DNA TagTamper
//! features and hints"), with encrypted PICC data and a CMAC:
//!
//! - `picc` — 16 bytes, AES-128-CBC with a zero IV under the SDM meta-read
//!   key: `tag ‖ UID(7) ‖ SDMReadCtr(3, LSB first) ‖ padding`, where `tag` has
//!   bit 7 set when the UID is mirrored, bit 6 when the counter is, and the UID
//!   length in its low nibble.
//! - `cmac` — 8 bytes: the odd-indexed bytes of AES-CMAC under the session
//!   key over the MAC input (empty when the MAC covers nothing but itself).
//!   The session key is AES-CMAC under the SDM file-read key of
//!   `3C C3 00 01 00 80 ‖ UID ‖ SDMReadCtr`.

use aes::cipher::{generic_array::GenericArray, BlockDecrypt, BlockEncrypt, KeyInit};
use aes::Aes128;

/// What a genuine tap says: which tag, and how many times it has been read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tap {
    pub uid: [u8; 7],
    pub counter: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub enum TapError {
    /// Not hex, or not the lengths SUN produces.
    Malformed,
    /// Decrypts, but not to a UID-and-counter record: wrong meta-read key, or
    /// not a tag configured this way.
    NotATag,
    /// The CMAC does not match: forged, altered, or the wrong file-read key.
    BadMac,
}

/// AES-CMAC (RFC 4493) over `msg`.
pub fn cmac(key: &[u8; 16], msg: &[u8]) -> [u8; 16] {
    let aes = Aes128::new(GenericArray::from_slice(key));
    let enc = |b: [u8; 16]| {
        let mut g = GenericArray::from(b);
        aes.encrypt_block(&mut g);
        <[u8; 16]>::from(g)
    };
    let dbl = |b: [u8; 16]| {
        let mut out = [0u8; 16];
        for i in 0..16 {
            out[i] = (b[i] << 1) | if i < 15 { b[i + 1] >> 7 } else { 0 };
        }
        if b[0] & 0x80 != 0 {
            out[15] ^= 0x87;
        }
        out
    };
    let k1 = dbl(enc([0u8; 16]));
    let k2 = dbl(k1);

    let blocks = msg.len().div_ceil(16).max(1);
    let complete = !msg.is_empty() && msg.len() % 16 == 0;
    let mut x = [0u8; 16];
    for i in 0..blocks {
        let mut block = [0u8; 16];
        let chunk = &msg[i * 16..msg.len().min(i * 16 + 16)];
        block[..chunk.len()].copy_from_slice(chunk);
        if i == blocks - 1 {
            if complete {
                for j in 0..16 {
                    block[j] ^= k1[j];
                }
            } else {
                block[chunk.len()] = 0x80;
                for j in 0..16 {
                    block[j] ^= k2[j];
                }
            }
        }
        for j in 0..16 {
            x[j] ^= block[j];
        }
        x = enc(x);
    }
    x
}

fn unhex<const N: usize>(s: &str) -> Result<[u8; N], TapError> {
    let s = s.trim();
    if s.len() != N * 2 {
        return Err(TapError::Malformed);
    }
    let mut out = [0u8; N];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).map_err(|_| TapError::Malformed)?;
    }
    Ok(out)
}

/// The UID and counter a tap's `picc` carries, decrypted under the meta-read key.
pub fn decrypt_picc(meta_key: &[u8; 16], picc_hex: &str) -> Result<Tap, TapError> {
    let enc: [u8; 16] = unhex(picc_hex)?;
    let aes = Aes128::new(GenericArray::from_slice(meta_key));
    let mut g = GenericArray::from(enc);
    aes.decrypt_block(&mut g); // one block, zero IV: CBC is ECB here
    let p = <[u8; 16]>::from(g);
    // UID and counter mirrored, a 7-byte UID.
    if p[0] & 0xC0 != 0xC0 || p[0] & 0x0F != 7 {
        return Err(TapError::NotATag);
    }
    let mut uid = [0u8; 7];
    uid.copy_from_slice(&p[1..8]);
    let counter = u32::from(p[8]) | u32::from(p[9]) << 8 | u32::from(p[10]) << 16;
    Ok(Tap { uid, counter })
}

/// The 8-byte SUN MAC for this tap over `mac_input`.
pub fn sun_mac(file_key: &[u8; 16], tap: &Tap, mac_input: &[u8]) -> [u8; 8] {
    let mut sv2 = [0u8; 16];
    sv2[..6].copy_from_slice(&[0x3C, 0xC3, 0x00, 0x01, 0x00, 0x80]);
    sv2[6..13].copy_from_slice(&tap.uid);
    sv2[13] = tap.counter as u8;
    sv2[14] = (tap.counter >> 8) as u8;
    sv2[15] = (tap.counter >> 16) as u8;
    let session = cmac(file_key, &sv2);
    let full = cmac(&session, mac_input);
    let mut out = [0u8; 8];
    for (i, o) in out.iter_mut().enumerate() {
        *o = full[2 * i + 1];
    }
    out
}

/// A tap, checked: decrypts `picc` and verifies `cmac` (constant-time).
pub fn verify(meta_key: &[u8; 16], file_key: &[u8; 16], picc_hex: &str, cmac_hex: &str, mac_input: &[u8]) -> Result<Tap, TapError> {
    let tap = decrypt_picc(meta_key, picc_hex)?;
    let given: [u8; 8] = unhex(cmac_hex)?;
    let want = sun_mac(file_key, &tap, mac_input);
    let diff = given.iter().zip(want.iter()).fold(0u8, |d, (a, b)| d | (a ^ b));
    if diff != 0 {
        return Err(TapError::BadMac);
    }
    Ok(tap)
}

/// What a tag configured with these keys would write for `tap`: its `picc`
/// and `cmac`, as hex. For tests and rehearsals without a physical tag.
pub fn emulate(meta_key: &[u8; 16], file_key: &[u8; 16], tap: &Tap, padding: [u8; 5]) -> (String, String) {
    let mut p = [0u8; 16];
    p[0] = 0xC7;
    p[1..8].copy_from_slice(&tap.uid);
    p[8] = tap.counter as u8;
    p[9] = (tap.counter >> 8) as u8;
    p[10] = (tap.counter >> 16) as u8;
    p[11..].copy_from_slice(&padding);
    let aes = Aes128::new(GenericArray::from_slice(meta_key));
    let mut g = GenericArray::from(p);
    aes.encrypt_block(&mut g);
    let hexs = |b: &[u8]| b.iter().map(|x| format!("{x:02X}")).collect::<String>();
    (hexs(&g), hexs(&sun_mac(file_key, tap, &[])))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h16(s: &str) -> [u8; 16] {
        unhex(s).unwrap()
    }

    #[test]
    fn cmac_matches_rfc_4493() {
        let k = h16("2b7e151628aed2a6abf7158809cf4f3c");
        let msg: Vec<u8> = (0..4)
            .flat_map(|i| unhex::<16>(["6bc1bee22e409f96e93d7e117393172a", "ae2d8a571e03ac9c9eb76fac45af8e51", "30c81c46a35ce411e5fbc1191a0a52ef", "f69f2445df4f9b17ad2b417be66c3710"][i]).unwrap())
            .collect();
        assert_eq!(cmac(&k, &[]), h16("bb1d6929e95937287fa37d129b756746"));
        assert_eq!(cmac(&k, &msg[..16]), h16("070a16b46b4d4144f79bdd9dd04a287c"));
        assert_eq!(cmac(&k, &msg[..40]), h16("dfa66747de9ae63030ca32611497c827"));
        assert_eq!(cmac(&k, &msg), h16("51f0bebf7e3b9d92fc49741779363cfe"));
    }

    /// NXP AN12196's example tap: all-zero keys.
    #[test]
    fn a_genuine_tap_verifies_and_names_its_tag() {
        let zero = [0u8; 16];
        let tap = verify(&zero, &zero, "EF963FF7828658A599F3041510671E88", "94EED9EE65337086", &[]).unwrap();
        assert_eq!(tap.uid, [0x04, 0xDE, 0x5F, 0x1E, 0xAC, 0xC0, 0x40]);
        assert_eq!(tap.counter, 0x3D);
    }

    #[test]
    fn an_emulated_tap_is_indistinguishable_from_the_real_one() {
        let zero = [0u8; 16];
        let tap = Tap { uid: [0x04, 0xDE, 0x5F, 0x1E, 0xAC, 0xC0, 0x40], counter: 0x3D };
        let (_, mac) = emulate(&zero, &zero, &tap, [0; 5]);
        assert_eq!(mac, "94EED9EE65337086", "the same MAC NXP's example tag wrote");
        let (picc, mac) = emulate(&[7; 16], &[9; 16], &Tap { uid: [4, 1, 2, 3, 4, 5, 6], counter: 70_000 }, [1, 2, 3, 4, 5]);
        assert_eq!(verify(&[7; 16], &[9; 16], &picc, &mac, &[]), Ok(Tap { uid: [4, 1, 2, 3, 4, 5, 6], counter: 70_000 }));
    }

    #[test]
    fn anything_else_is_refused() {
        let zero = [0u8; 16];
        assert_eq!(verify(&zero, &zero, "EF963FF7828658A599F3041510671E88", "94EED9EE65337087", &[]), Err(TapError::BadMac));
        let mut other = [0u8; 16];
        other[0] = 1;
        assert_eq!(verify(&zero, &other, "EF963FF7828658A599F3041510671E88", "94EED9EE65337086", &[]), Err(TapError::BadMac), "another file-read key");
        assert!(verify(&other, &zero, "EF963FF7828658A599F3041510671E88", "94EED9EE65337086", &[]).is_err(), "another meta-read key");
        assert_eq!(verify(&zero, &zero, "EF96", "94EED9EE65337086", &[]), Err(TapError::Malformed));
        assert_eq!(verify(&zero, &zero, "zz963FF7828658A599F3041510671E88", "94EED9EE65337086", &[]), Err(TapError::Malformed));
    }
}
