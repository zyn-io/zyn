//! The drand endpoint, against the real service.
//!
//! This exists because the URL was wrong and every stubbed test passed anyway.
//! A transport double proves the parsing and the verification; it cannot prove
//! the address is right, and an unreachable beacon at reveal time is a sale
//! that cannot be drawn.
//!
//! Ignored by default so a network outage is not a failing build. Run it
//! deliberately, and before any reveal:
//!
//! ```text
//! cargo test -p zynzapd --test drand_live -- --ignored --nocapture
//! ```

#[test]
#[ignore = "needs the network; run before a reveal"]
fn a_real_quicknet_round_fetches_and_verifies() {
    let get = |url: &str| -> Result<Vec<u8>, String> {
        let r = ureq::get(url)
            .timeout(std::time::Duration::from_secs(20))
            .call()
            .map_err(|e| e.to_string())?;
        let mut b = Vec::new();
        std::io::Read::read_to_end(&mut r.into_reader(), &mut b).map_err(|e| e.to_string())?;
        Ok(b)
    };

    // A round long past, so the answer is stable forever rather than depending
    // on when the test runs.
    let round = 1_000_000u64;
    let beacon = zynzapd::reveal::beacon(round, cave_randomness::QUICKNET_CHAIN_HASH, &get)
        .expect("a past Quicknet round must fetch and verify");
    assert_eq!(beacon.round, round);
    assert_eq!(beacon.signature.len(), 48, "BLS on G1");
    assert_eq!(beacon.chain_hash, cave_randomness::QUICKNET_CHAIN_HASH);

    // And the verification is real: one flipped bit must fail it.
    let mut tampered = beacon.clone();
    tampered.signature[0] ^= 1;
    assert!(
        cave_randomness::verify_quicknet_beacon(&tampered).is_err(),
        "a tampered signature must not verify"
    );
}
