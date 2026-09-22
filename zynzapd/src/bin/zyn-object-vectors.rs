//! Test vectors for the object ledger's note format.
//!
//! Two implementations now open the same notes: `zyn-vm` in Rust and
//! `apps/nap-web/src/object.js` in the browser. Nothing makes them agree
//! except that they were written from the same spec, and the failure mode if
//! they drift is silent — a wallet looks at the right chain, opens nothing,
//! and reports zero, which is indistinguishable from having been robbed.
//!
//! So both sides test against one fixture. This prints it; the fixture is
//! committed, and a test on each side checks its own implementation against
//! it. Neither side can move alone.

use zyn_vm::note_enc::{seal, viewing_public, StaticSecret};
use zyn_vm::object::{commitment, nullifier, Note};
use zyn_vm::Fixed;

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

fn main() {
    // Fixed inputs, so the fixture is reproducible.
    let seed = b"a seed that is not a real seed";
    let viewing = StaticSecret::from(zynzapd::object_keys::viewing_secret(seed, 0).to_bytes());
    let nkey = zynzapd::object_keys::nullifier_key(seed, 0);
    let owner = [0xA1u8; 32];

    let note = Note {
        asset: [0x7a; 32],
        amount: Fixed::whole(3),
        owner,
        nonce: [0x5e; 32],
    };
    let mut plain = Vec::with_capacity(80);
    plain.extend_from_slice(&note.asset);
    plain.extend_from_slice(&note.amount.0.to_be_bytes());
    plain.extend_from_slice(&note.nonce);

    let enc = seal(
        &viewing_public(&viewing),
        StaticSecret::from([0x99u8; 32]),
        &plain,
    )
    .expect("seals");
    let commit = commitment(&note).expect("a positive amount and a non-zero nonce");

    println!("{{");
    println!("  \"seed_utf8\": \"{}\",", String::from_utf8_lossy(seed));
    println!("  \"account\": 0,");
    println!("  \"viewing_key\": \"{}\",", hex(&viewing_public(&viewing)));
    println!("  \"nullifier_key\": \"{}\",", hex(&nkey));
    println!("  \"owner\": \"{}\",", hex(&owner));
    println!("  \"note\": {{");
    println!("    \"asset\": \"{}\",", hex(&note.asset));
    println!("    \"amount\": \"{}\",", note.amount.0);
    println!("    \"nonce\": \"{}\"", hex(&note.nonce));
    println!("  }},");
    println!("  \"epk\": \"{}\",", hex(&enc.epk));
    println!("  \"ciphertext\": \"{}\",", hex(&enc.ciphertext));
    println!("  \"commitment\": \"{}\",", hex(&commit));
    println!("  \"nullifier\": \"{}\"", hex(&nullifier(&nkey, &commit)));
    println!("}}");
}
