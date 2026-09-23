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
    println!("  \"nullifier\": \"{}\",", hex(&nullifier(&nkey, &commit)));

    // The program ids and intent tags a browser needs in order to *build* an
    // intent rather than only read one. Wire constants: a wallet that encoded
    // a different tag would have its submission refused, and a wrong program
    // id is an intent the VM will not run.
    println!("  \"programs\": {{");
    println!(
        "    \"register_viewing_key\": \"{}\",",
        hex(&zyn_vm::program::register_viewing_key().0)
    );
    println!("    \"private_transfer\": \"{}\",", hex(&zyn_vm::program::private_transfer().0));
    println!(
        "    \"place_offer\": \"{}\",",
        hex(&zyn_vm::program::place_offer().0)
    );
    println!(
        "    \"take_offer\": \"{}\"",
        hex(&zyn_vm::program::take_offer().0)
    );
    println!("  }},");
    println!("  \"intent_tags\": {{ \"spend\": 2, \"register\": 8 }},");

    // A whole Register intent, encoded, so the browser can be held to the
    // exact bytes rather than to a description of them.
    let reg = <objectvm::ObjectVm as zyn_vm::spec::MicrochainVm>::encode_intent(&objectvm::Op::Register {
        account: owner,
        program: zyn_vm::program::register_viewing_key(),
        viewing: viewing_public(&viewing),
    });
    println!("  \"register_intent\": \"{}\",", hex(&reg));

    // A whole Spend: one input retired, one output created. The shape a send
    // takes, pinned so the browser can be held to the bytes.
    let out_note = Note {
        asset: note.asset,
        amount: Fixed::whole(2),
        owner: [0xB0; 32],
        nonce: [0x77; 32],
    };
    let mut out_plain = Vec::with_capacity(80);
    out_plain.extend_from_slice(&out_note.asset);
    out_plain.extend_from_slice(&out_note.amount.0.to_be_bytes());
    out_plain.extend_from_slice(&out_note.nonce);
    let out_enc = seal(
        &viewing_public(&StaticSecret::from([0xB0u8; 32])),
        StaticSecret::from([0x55u8; 32]),
        &out_plain,
    )
    .expect("seals");
    let spend = <objectvm::ObjectVm as zyn_vm::spec::MicrochainVm>::encode_intent(
        &objectvm::Op::Spend {
            spender: owner,
            program: zyn_vm::program::private_transfer(),
            inputs: vec![(commit, nullifier(&nkey, &commit))],
            outputs: vec![objectvm::Output {
                note: out_note,
                enc: out_enc.clone(),
            }],
        },
    );
    println!("  \"spend\": {{");
    println!("    \"out_owner\": \"{}\",", hex(&out_note.owner));
    println!("    \"out_amount\": \"{}\",", out_note.amount.0);
    println!("    \"out_nonce\": \"{}\",", hex(&out_note.nonce));
    println!("    \"out_epk\": \"{}\",", hex(&out_enc.epk));
    println!("    \"out_ciphertext\": \"{}\",", hex(&out_enc.ciphertext));
    println!("    \"intent\": \"{}\"", hex(&spend));
    println!("  }},");

    // A signed frame, exactly as `zyn_submitSigned` takes it. Ed25519 is
    // deterministic, so a browser signing the same payload with the same key
    // must produce these bytes exactly -- which is what the test holds it to.
    use ed25519_dalek::{Signer, SigningKey};
    let key = SigningKey::from_bytes(&[7u8; 32]);
    let account = zyn_vm::auth::account_of(zyn_vm::auth::Scheme::Ed25519, key.verifying_key().as_bytes());
    let intent = objectvm::Op::Register {
        account,
        program: zyn_vm::program::register_viewing_key(),
        viewing: viewing_public(&viewing),
    };
    let auth = zyn_vm::auth::Authorization {
        chain_id: 212,
        vm_id: zyn_vm::zvm::vm_id::<objectvm::ObjectVm>(),
        valid_until_epoch: 1000,
    };
    let payload = zyn_vm::auth::signing_payload::<objectvm::ObjectVm>(&auth, &intent);
    let sig = key.sign(&payload);
    let authorized = zyn_vm::verify::authorize_intent::<objectvm::ObjectVm>(
        &[zyn_vm::verify::Credential::Ed25519 { key: key.verifying_key().to_bytes(), signature: sig.to_bytes() }],
        &auth,
        intent,
        &<objectvm::ObjectVm as zyn_vm::spec::MicrochainVm>::genesis(212, objectvm::Limits { max_io: 8 }),
    )
    .expect("a holder's own signature authorizes their own registration");
    println!("  \"signed\": {{");
    println!("    \"seed\": \"{}\",", hex(&[7u8; 32]));
    println!("    \"account\": \"{}\",", hex(&account));
    println!("    \"chain\": 212,");
    println!("    \"valid_until_epoch\": 1000,");
    println!("    \"vm_id\": \"{}\",", hex(&auth.vm_id));
    println!("    \"payload\": \"{}\",", hex(&payload));
    println!("    \"frame\": \"{}\"", hex(&zyn_vm::verify::encode_authorized::<objectvm::ObjectVm>(&authorized)));
    println!("  }},");

    // The market: program ids, derived ids, and one of every intent, with a
    // fixed change note so the browser can rebuild each byte for byte.
    let c = zynzapd::market_vectors::change();
    println!("  \"market\": {{");
    println!("    \"zec\": \"{}\",", hex(&zynzapd::market_vectors::ZEC));
    println!("    \"token\": \"{}\",", hex(&zynzapd::market_vectors::TOKEN));
    println!("    \"owner\": \"{}\",", hex(&zynzapd::market_vectors::OWNER));
    println!("    \"nonce\": \"{}\",", hex(&zynzapd::market_vectors::NONCE));
    println!("    \"change_nonce\": \"{}\",", hex(&c.note.nonce));
    println!("    \"change_epk\": \"{}\",", hex(&c.enc.epk));
    println!("    \"change_ciphertext\": \"{}\",", hex(&c.enc.ciphertext));
    let v = zynzapd::market_vectors::vectors();
    for (i, (k, val)) in v.iter().enumerate() {
        let comma = if i + 1 == v.len() { "" } else { "," };
        println!("    \"{k}\": \"{val}\"{comma}");
    }
    println!("  }}");
    println!("}}");
}
