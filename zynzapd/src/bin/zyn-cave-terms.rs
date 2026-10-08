//! Collection Z's sale policy and terms, assembled from what was decided.
//!
//! Every value here was settled somewhere else and is repeated rather than
//! chosen: quantity on 9 Sep, prices in §64.17, the manifest root by building
//! it, the close rule in §100.8. What did not exist was anything that put them
//! in one object and printed a digest — so the terms could not be published,
//! and until terms are published nothing may be sold (§64.16).
//!
//! It prints. It does not broadcast, sign, or write to the chain: publishing is
//! a deliberate act and this is the thing you read before taking it.
//!
//!   zyn-cave-terms                 print the policy and terms (v2), with digests
//!   zyn-cave-terms bytes           v2's canonical bytes, as hex
//!   zyn-cave-terms bytes-v1        v1's (superseded, §175)
//!   zyn-cave-terms announce <H>    the close announcement for close height H


use cave_randomness::CloseAnnouncement;
use zynzapd::cave_z::{policy, terms, terms_v2};

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

fn main() {
    let p = policy();
    if let Err(e) = p.validate() {
        eprintln!("zyn-cave-terms: the policy does not validate: {e}");
        std::process::exit(1);
    }
    let t = terms(&p);
    if let Err(e) = t.check(&p) {
        eprintln!("zyn-cave-terms: the terms do not agree with the policy: {e}");
        std::process::exit(1);
    }

    let mut args = std::env::args().skip(1);
    if args.next().as_deref() == Some("announce") {
        let close: u32 = args
            .next()
            .and_then(|h| h.parse().ok())
            .unwrap_or_else(|| {
                eprintln!("zyn-cave-terms announce <close-height>");
                std::process::exit(2);
            });
        // Built, not published: there is no proof yet, because publishing it is
        // what creates one. `resolve_unproven` is the honest call here and is
        // named so that using it anywhere else is visible (§101.4).
        let a = CloseAnnouncement {
            policy_digest: t.policy_digest,
            // Filled in by whoever broadcasts: the tip and round at that moment.
            announced_at_zcash_height: 0,
            announced_at_drand_round: 0,
            deposit_close_zcash_height: close,
        };
        println!("close announcement for height {close}");
        println!("  policy digest   {}", hex(&a.policy_digest));
        println!("  fill in announced_at_zcash_height and announced_at_drand_round");
        println!("  at broadcast, then publish digest(announcement) in a ZYP memo");
        println!("  reveal will be   close + {} blocks", 100);
        println!("  drand round      announced_round + {}", 2_400);
        return;
    }

    // The canonical bytes themselves, for publication. A digest nobody can
    // recompute is a number, not a commitment — so the bytes have to be
    // publishable, not only their hash.
    let v2 = terms_v2(&p);
    match std::env::args().nth(1).as_deref() {
        Some("bytes") => return println!("{}", hex(&v2.canonical_bytes())),
        Some("bytes-v1") => return println!("{}", hex(&t.canonical_bytes())),
        _ => {}
    }

    println!("Cave Collection Z — sale policy and terms");
    println!();
    println!("collection CAVEZ   {}", hex(&p.collection_id));
    println!("supply                {}", p.supply);
    println!("manifest root         {}", hex(&p.manifest_root));
    println!("quantity              weighted 1:80  2:19  3:1");
    println!("identity              concealed shuffle");
    println!("close                 announced, >= 1152 blocks' notice");
    println!("reveal                close + 100 blocks, announced round + 2400");
    println!("treasury              10% to the collection's DAO treasury (no key spends it)");
    println!(
        "prices (zatoshi)      entry {}  claim {}  open mint {}",
        t.entry_price_zatoshi, t.claim_price_zatoshi, t.open_mint_price_zatoshi
    );
    println!(
        "whitelist             listed accounts only; ticket n > {} costs {} more per ticket past it",
        v2.gate_base, v2.gate_step_zatoshi
    );
    println!("confirmations         {}", t.min_confirmations);
    println!("claim window          {} blocks", t.claim_window_blocks);
    let (pool, treasury) = t.split(t.entry_price_zatoshi);
    println!("split of one entry    {} to the pool, {} to the DAO treasury", pool, treasury);
    println!(
        "DAO                   {} backers, {}-block windows, {} bps quorum",
        v2.dao_backers, v2.dao_window_blocks, v2.dao_quorum_bps
    );
    println!(
        "burn tickets          {} zatoshi of the token each, median of {} s (>= {} readings); <= {} per burn, {} in all",
        v2.burn_ticket_zatoshi, v2.burn_median_seconds, v2.burn_min_readings, v2.burn_per_burn, v2.burn_cap
    );
    println!();
    println!("POLICY DIGEST  {}", hex(&t.policy_digest));
    println!("TERMS DIGEST   {}  (cave-sale-terms/v2)", hex(&v2.digest()));
    println!("SUPERSEDES     {}  (v1)", hex(&v2.supersedes));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The values are settled elsewhere; this asserts they were transcribed,
    /// not re-decided. A digest is only worth publishing if the inputs are the
    /// ones people agreed to.
    #[test]
    fn the_published_values_are_the_recorded_ones() {
        let p = policy();
        p.validate().expect("valid");
        assert_eq!(hex(&p.collection_id), "024277b607ce4edbf78fe2855aba562096d00264729168bd0ddea71bdf5b5f1e");
        assert_eq!(
            hex(&p.manifest_root),
            "0c56593e1b16bd513a4c88baf31d42602acd89db85aef2b0dcbc4e4bce108426"
        );
        assert_eq!(p.supply, 4_444);
        // The draw recorded on 9 Sep, and the reservation it forces.
        assert_eq!(p.maximum_receipt(), 3);
        let t = terms(&p);
        t.check(&p).expect("terms agree");
        assert_eq!(
            (t.entry_price_zatoshi, t.claim_price_zatoshi, t.open_mint_price_zatoshi),
            (1_000_000, 500_000, 1_330_000),
            "§64.17, frozen"
        );
        assert_eq!(t.split(1_000_000), (900_000, 100_000), "90 to the pool, 10 to the treasury");
        assert_eq!(t.treasury, swapvm::types::TREASURY_26460);
    }

    /// An announced close means the policy must *not* carry reveal targets;
    /// this is the assertion that would catch someone filling them in later.
    #[test]
    fn the_close_is_announced_and_the_reveal_is_derived() {
        let p = policy();
        assert_eq!(p.deposit_close_zcash_height, 0);
        assert!(p.randomness.reveal_zcash_height.is_none());
        assert!(p.randomness.reveal_drand_round.is_none());
        let rule = p.randomness.announced_close.expect("announced");
        assert_eq!(rule.reveal_offset_blocks, 100);
        assert_eq!(rule.min_notice_blocks, 1_152);
    }

    /// Both digests are deterministic, or publishing one means nothing.
    #[test]
    fn the_digests_are_stable_across_builds() {
        let a = terms(&policy());
        let b = terms(&policy());
        assert_eq!(a.digest(), b.digest());
        assert_eq!(a.policy_digest, b.policy_digest);
    }
}
