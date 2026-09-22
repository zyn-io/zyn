//! The fixture both implementations are held to.
//!
//! `apps/nap-web/tests/object.test.mjs` checks the browser against this file.
//! This checks Rust against the same file. Neither side can move alone, which
//! is the only thing that catches a drift whose symptom is silence — a wallet
//! looking at the right chain, opening nothing, and reporting zero.

use zyn_vm::note_enc::{open, seal, viewing_public, StaticSecret};
use zyn_vm::object::{commitment, nullifier, Note};
use zyn_vm::Fixed;

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap())
        .collect()
}

/// `"key": "value"` out of the fixture, without a JSON dependency.
fn field(json: &str, key: &str) -> String {
    let at = json
        .find(&format!("\"{key}\""))
        .unwrap_or_else(|| panic!("the fixture has no {key}"));
    let rest = &json[at + key.len() + 2..];
    let open = rest.find('"').expect("a value");
    let rest = &rest[open + 1..];
    rest[..rest.find('"').expect("a closing quote")].to_string()
}

fn fixture() -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../apps/nap-web/tests/object-vectors.json"),
    )
    .expect("the fixture is committed beside the browser test that reads it")
}

#[test]
fn the_rust_side_still_produces_the_pinned_vectors() {
    let j = fixture();
    let seed = field(&j, "seed_utf8");
    let seed = seed.as_bytes();

    let viewing = StaticSecret::from(zynzapd::object_keys::viewing_secret(seed, 0).to_bytes());
    let nkey = zynzapd::object_keys::nullifier_key(seed, 0);

    assert_eq!(
        hex(&viewing_public(&viewing)),
        field(&j, "viewing_key"),
        "the viewing derivation moved; every registered account is bound to it"
    );
    assert_eq!(hex(&nkey), field(&j, "nullifier_key"));

    let owner: [u8; 32] = unhex(&field(&j, "owner")).try_into().unwrap();
    let note = Note {
        asset: unhex(&field(&j, "asset")).try_into().unwrap(),
        amount: Fixed(field(&j, "amount").parse().unwrap()),
        owner,
        nonce: unhex(&field(&j, "nonce")).try_into().unwrap(),
    };

    let commit = commitment(&note).expect("valid");
    assert_eq!(
        hex(&commit),
        field(&j, "commitment"),
        "the commitment encoding moved; the chain's published commitments would not match"
    );
    assert_eq!(hex(&nullifier(&nkey, &commit)), field(&j, "nullifier"));
}

/// The ciphertext in the fixture must still open here, byte for byte. Sealing
/// is randomised per ephemeral key, so this checks the *decrypt* path against
/// bytes that were frozen rather than regenerating them.
#[test]
fn the_pinned_ciphertext_still_opens() {
    let j = fixture();
    let seed = field(&j, "seed_utf8");
    let viewing = StaticSecret::from(
        zynzapd::object_keys::viewing_secret(seed.as_bytes(), 0).to_bytes(),
    );
    let enc = zyn_vm::note_enc::EncryptedNote {
        epk: unhex(&field(&j, "epk")).try_into().unwrap(),
        ciphertext: unhex(&field(&j, "ciphertext")),
    };
    let plain = open(&viewing, &enc).expect("the pinned note must still open");
    assert_eq!(plain.len(), 80, "asset(32) + amount(16) + nonce(32)");
    assert_eq!(hex(&plain[..32]), field(&j, "asset"));
    assert_eq!(hex(&plain[48..]), field(&j, "nonce"));
}

/// The program ids and the encoded intent in the fixture must still be what
/// Rust produces. The browser builds intents from these bytes, so a change
/// here that the fixture did not catch would mean a wallet submitting
/// something the node refuses -- or worse, something it accepts that runs a
/// different rule.
#[test]
fn the_pinned_programs_and_intent_encoding_still_hold() {
    let j = fixture();
    assert_eq!(
        hex(&zyn_vm::program::register_viewing_key().0),
        field(&j, "register_viewing_key")
    );

    let seed = field(&j, "seed_utf8");
    let viewing = StaticSecret::from(
        zynzapd::object_keys::viewing_secret(seed.as_bytes(), 0).to_bytes(),
    );
    let owner: [u8; 32] = unhex(&field(&j, "owner")).try_into().unwrap();
    let encoded = <objectvm::ObjectVm as zyn_vm::spec::MicrochainVm>::encode_intent(
        &objectvm::Op::Register {
            account: owner,
            program: zyn_vm::program::register_viewing_key(),
            viewing: viewing_public(&viewing),
        },
    );
    assert_eq!(hex(&encoded), field(&j, "register_intent"));

    // The Spend encoding too: it carries lengths and a variable-length
    // ciphertext, so it is the one a second implementation is most likely to
    // get subtly wrong.
    let nkey = zynzapd::object_keys::nullifier_key(seed.as_bytes(), 0);
    let note = Note {
        asset: unhex(&field(&j, "asset")).try_into().unwrap(),
        amount: Fixed(field(&j, "amount").parse().unwrap()),
        owner,
        nonce: unhex(&field(&j, "nonce")).try_into().unwrap(),
    };
    let commit = commitment(&note).unwrap();
    let spend = <objectvm::ObjectVm as zyn_vm::spec::MicrochainVm>::encode_intent(
        &objectvm::Op::Spend {
            spender: owner,
            program: zyn_vm::program::private_transfer(),
            inputs: vec![(commit, nullifier(&nkey, &commit))],
            outputs: vec![objectvm::Output {
                note: Note {
                    asset: note.asset,
                    amount: Fixed(field(&j, "out_amount").parse().unwrap()),
                    owner: unhex(&field(&j, "out_owner")).try_into().unwrap(),
                    nonce: unhex(&field(&j, "out_nonce")).try_into().unwrap(),
                },
                enc: zyn_vm::note_enc::EncryptedNote {
                    epk: unhex(&field(&j, "out_epk")).try_into().unwrap(),
                    ciphertext: unhex(&field(&j, "out_ciphertext")),
                },
            }],
        },
    );
    assert_eq!(hex(&spend), field(&j, "intent"));
}

/// And sealing to the same key still produces something that round-trips —
/// the encrypt path, which the frozen ciphertext cannot cover.
#[test]
fn sealing_and_opening_still_round_trips() {
    let seed = b"a seed that is not a real seed";
    let viewing = StaticSecret::from(zynzapd::object_keys::viewing_secret(seed, 0).to_bytes());
    let body = vec![7u8; 80];
    let enc = seal(
        &viewing_public(&viewing),
        StaticSecret::from([0x42u8; 32]),
        &body,
    )
    .unwrap();
    assert_eq!(open(&viewing, &enc).unwrap(), body);
}
