//! Does the code do what the published terms say?
//!
//! The terms commit `weighted 1:80 2:19 3:1`, `concealed shuffle`, and a close
//! that is *announced* rather than fixed. `tests/sale_pipeline.rs` proved the
//! pipeline composes — but with `Fixed { items: 1 }`, `Sequential`, and a close
//! committed in the policy. That is a different path through the allocator than
//! the one Collection Z actually sells under, and none of it was exercised.
//!
//! This drives the real `cave_z::policy()`.

use cave_randomness::{DrandBeacon, QUICKNET_CHAIN_HASH};
use zyn_custody::memo::{CavePaymentMemo, CavePaymentPurpose};
use zynzapd::cave_z;
use zynzapd::draw::{self, Chain, Plan};
use zynzapd::reveal::Sighting;

/// Real Quicknet rounds, so the beacon verification is the real one.
const ANNOUNCED_ROUND: u64 = 5_000_000;
const REVEAL_ROUND: u64 = 5_002_400; // announced + the committed 2,400
const REVEAL_SIG: &str = "83559dd072b76a12aaf0b3f902f24a01c731307878d424fd095986f8de75f2687d0a1489aaa72432d74366ae31591e1f";

const ANNOUNCED_AT: u32 = 3_400_000;
const CLOSE: u32 = 3_402_000; // 2,000 blocks of notice, over the committed 1,152
const REVEAL_HEIGHT: u32 = CLOSE + 100;

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap())
        .collect()
}

fn manifest() -> Vec<cave_randomness::ManifestEntry> {
    let raw = std::fs::read_to_string("../examples/zyn-pfp-concepts/out/content.json")
        .or_else(|_| std::fs::read_to_string("examples/zyn-pfp-concepts/out/content.json"))
        .expect("the real Collection Z manifest");
    let doc: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let mut content = std::collections::BTreeMap::new();
    for (k, v) in doc.as_object().unwrap() {
        let serial: u32 = k.parse().unwrap();
        let hash = unhex(v.get("content").unwrap().as_str().unwrap());
        content.insert(serial, <[u8; 32]>::try_from(&hash[..]).unwrap());
    }
    cave_z::manifest_from(&content)
}

struct Fake {
    seen: Vec<Sighting>,
}

impl Chain for Fake {
    fn tip(&self) -> Result<u64, String> {
        Ok(REVEAL_HEIGHT as u64 + 10)
    }
    fn cave_payments(&self, from: u64, to: u64) -> Result<(u64, Vec<Sighting>), String> {
        Ok((
            to,
            self.seen
                .iter()
                .filter(|s| s.height >= from && s.height <= to)
                .copied()
                .collect(),
        ))
    }
    fn block_hash(&self, _height: u64) -> Result<[u8; 32], String> {
        Ok([0x5b; 32])
    }
}

fn paid(txid: u8, who: u8, terms: [u8; 32]) -> Sighting {
    Sighting {
        txid: [txid; 32],
        height: (CLOSE - 500) as u64,
        tx_index: txid as u32,
        output_index: 0,
        amount_zatoshi: 1_000_000, // the committed entry price
        payment: CavePaymentMemo {
            purpose: CavePaymentPurpose::Entry,
            reference: terms,
            recipient: [who; 32],
        },
    }
}

/// The reveal beacon a caller would have fetched and verified.
fn reveal_beacon() -> DrandBeacon {
    DrandBeacon {
        chain_hash: QUICKNET_CHAIN_HASH,
        round: REVEAL_ROUND,
        signature: unhex(REVEAL_SIG),
    }
}

#[test]
fn the_published_policy_can_actually_be_drawn() {
    let policy = cave_z::policy();
    policy.validate().expect("the published policy validates");
    let terms = cave_z::terms(&policy).digest();

    let announcement = cave_randomness::CloseAnnouncement {
        policy_digest: policy.digest().unwrap(),
        announced_at_zcash_height: ANNOUNCED_AT,
        announced_at_drand_round: ANNOUNCED_ROUND,
        deposit_close_zcash_height: CLOSE,
    };
    let proof = cave_randomness::PublicationProof {
        txid: [0x11; 32],
        height: ANNOUNCED_AT,
    };
    let plan = Plan::from_announcement(
        &policy,
        &announcement,
        &proof,
        cave_z::CONFIRMATIONS,
        (CLOSE - 2_000) as u64,
        terms,
    )
    .expect("the announcement resolves");

    assert_eq!(plan.reveal_drand_round, REVEAL_ROUND, "derived reveal round");
    assert_eq!(plan.reveal_zcash_height, REVEAL_HEIGHT, "derived reveal height");

    let chain = Fake {
        seen: vec![paid(1, 0xa1, terms), paid(2, 0xb2, terms)],
    };
    // The beacon is the real one for that round, handed in rather than fetched,
    // so this test does not depend on drand being up.
    let beacon = reveal_beacon();
    let get = move |_: &str| -> Result<Vec<u8>, String> {
        Ok(format!(
            r#"{{"round":{},"signature":"{}"}}"#,
            beacon.round,
            REVEAL_SIG
        )
        .into_bytes())
    };

    let outcome = draw::execute(&plan, &policy, &manifest(), &chain, &get)
        .expect("the sale Collection Z actually publishes must be drawable");

    assert_eq!(outcome.run.tickets, 2, "two paid entries, two tickets");
    assert!(outcome.refused.is_empty(), "both paid the committed price");
    let a = outcome.run.serials_for(&[0xa1; 32]);
    let b = outcome.run.serials_for(&[0xb2; 32]);
    assert!(!a.is_empty() && !b.is_empty(), "both buyers drew something");
    assert!(a.iter().all(|s| !b.contains(s)), "no serial in two accounts");
}

/// The terms say `weighted 1:80  2:19  3:1`. That is a claim about what buyers
/// get, so it is worth measuring rather than reading off the policy struct.
#[test]
fn the_draw_produces_the_weights_the_terms_publish() {
    let policy = cave_z::policy();
    let terms = cave_z::terms(&policy).digest();
    let announcement = cave_randomness::CloseAnnouncement {
        policy_digest: policy.digest().unwrap(),
        announced_at_zcash_height: ANNOUNCED_AT,
        announced_at_drand_round: ANNOUNCED_ROUND,
        deposit_close_zcash_height: CLOSE,
    };
    let proof = cave_randomness::PublicationProof { txid: [0x11; 32], height: ANNOUNCED_AT };
    let plan = Plan::from_announcement(
        &policy, &announcement, &proof, cave_z::CONFIRMATIONS, (CLOSE - 2_000) as u64, terms,
    )
    .unwrap();

    // 2,000 entries: enough for the weights to show, and 2,000 x ~1.21 items
    // still fits inside the 4,444 supply.
    let mut seen = Vec::new();
    for i in 0..2_000u32 {
        let mut s = paid((i % 251) as u8 + 1, (i % 199) as u8 + 1, terms);
        s.txid = {
            let mut t = [0u8; 32];
            t[..4].copy_from_slice(&i.to_be_bytes());
            t[31] = 0xe1;
            t
        };
        s.tx_index = i;
        s.height = (CLOSE - 1_000 + (i % 400)) as u64;
        s.payment.recipient = {
            let mut a = [0u8; 32];
            a[..4].copy_from_slice(&i.to_be_bytes());
            a[31] = 0xac;
            a
        };
        seen.push(s);
    }

    let beacon = reveal_beacon();
    let get = move |_: &str| -> Result<Vec<u8>, String> {
        Ok(format!(r#"{{"round":{},"signature":"{}"}}"#, beacon.round, REVEAL_SIG).into_bytes())
    };
    let outcome = draw::execute(&plan, &policy, &manifest(), &Fake { seen }, &get)
        .expect("2,000 entries draw");

    assert_eq!(outcome.run.tickets, 2_000);
    let mut count = [0usize; 4];
    for receipt in &outcome.run.batch.receipts {
        count[receipt.quantity as usize] += 1;
    }
    let total = 2_000f64;
    let one = count[1] as f64 / total * 100.0;
    let two = count[2] as f64 / total * 100.0;
    let three = count[3] as f64 / total * 100.0;
    println!("measured over 2,000 entries: 1 item {one:.1}%  2 items {two:.1}%  3 items {three:.1}%");

    assert_eq!(count[0], 0, "no entry may draw nothing");
    // Generous bands: this is one sample of a real beacon, not a chi-squared
    // test. What it catches is weights wired in the wrong order or ignored.
    assert!((76.0..84.0).contains(&one), "1 item should be ~80%, measured {one:.1}%");
    assert!((15.5..23.0).contains(&two), "2 items should be ~19%, measured {two:.1}%");
    assert!((0.2..3.0).contains(&three), "3 items should be ~1%, measured {three:.1}%");
}
