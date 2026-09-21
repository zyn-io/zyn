//! The browser wallet encodes its own intents; the chain decodes them.
//!
//! `apps/nap-web` signs offers locally and relays the frame through
//! `zyn_sendRawIntent` — it holds no agent and cannot ask Rust to encode
//! anything. So the same field order exists in two languages, and a signature
//! commits to the bytes: a field out of order is not a decode error, it is a
//! valid signature over a different instruction.
//!
//! These are the bytes `apps/nap-web/src/core.js` produces for fixed inputs.
//! The three assets differ on purpose — identical ones would hide a swap.

use swapvm::tx::Intent;
use swapvm::types::AssetId;
use swapvm::wire;
use swapvm::Fixed;

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

const WAD: i128 = 1_000_000_000_000_000_000;

#[test]
fn the_browser_wallet_encodes_offers_the_way_the_chain_reads_them() {
    let maker = [0xa1u8; 32];
    let offer_asset: AssetId = [0xb2u8; 32];
    let want_asset: AssetId = [0xc3u8; 32];

    let place = Intent::PlaceOffer {
        maker,
        offer_asset,
        offer_amount: Fixed(WAD),
        want_asset,
        want_amount: Fixed(WAD * 2),
        expires_at_epoch: 12_345,
    };
    let take = Intent::TakeOffer { taker: maker, offer: 777 };
    let cancel = Intent::CancelOffer { maker, offer: 777 };

    for (intent, expected) in [
        (place, "23a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b200000000000000000de0b6b3a7640000c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c300000000000000001bc16d674ec800000000000000003039"),
        (take, "24a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a10000000000000309"),
        (cancel, "25a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a10000000000000309"),
    ] {
        let mut e = zyn_vm::commit::Encoder::new();
        wire::encode_intent(&mut e, &intent);
        assert_eq!(
            hex(e.finish()),
            expected,
            "apps/nap-web encodes this intent differently from the chain"
        );
    }
}
