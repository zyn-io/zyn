//! Where an entry's money actually goes, over a real socket.
//!
//! The terms promise two things about money and nothing tested either: that
//! 90% of an entry funds the collection's pool, and that redeeming an item pays
//! out of that pool. Until §114 the sale moved nothing at all — a buyer kept
//! their ZEC.zy *and* got an item — and no test noticed, because every test
//! stopped at the allocation.
//!
//! This drives the path the service drives: the buyer signs, the gateway relays
//! the bytes untouched (`submit_raw`), the treasury funds the pool, and the
//! collection goes live so an item can be handed back for its share.

use std::net::{SocketAddr, TcpListener};
use std::sync::{Arc, Mutex};

use cave_randomness::SaleTerms;
use ed25519_dalek::SigningKey;
use swapvm::state::SwapState;
use swapvm::tx::{Intent, Receipt};
use swapvm::types::XZEC;
use swapvm::{Fixed, Params};
use zyn::epoch::{Economics, EpochPolicy};
use zyn::node::Node;
use zynzapd::client::{frame_submission, Node as RpcNode};

const CHAIN: u32 = 7;
/// One zatoshi in Fixed's 1e18 scale.
const ZAT: i128 = 10_000_000_000;

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn account_of(k: &SigningKey) -> [u8; 32] {
    zyn_vm::auth::account_of(zyn_vm::auth::Scheme::Ed25519, k.verifying_key().as_bytes())
}

fn start(node: Node<SwapState>) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().unwrap();
    let server = Arc::new(zynzapd::rpc::Server {
        node: Arc::new(Mutex::new(node)),
        chain_id: CHAIN,
        now: || 0,
        health: Arc::new(Mutex::new(Vec::new())),
        inbox: Arc::new(Mutex::new(zynzapd::rpc::Inbox::default())),
        replica: None,
        replay: Arc::new(Mutex::new(zyn::replay::ReplayIndex::default())),
        deposits: None,
        da_dir: None,
    });
    std::thread::spawn(move || zynzapd::rpc::serve(server, listener));
    addr
}

/// A chain where the buyer's Zcash deposit has already been credited — which
/// is the state `/settle` meets, not one it creates.
fn chain_with_a_credited_buyer(buyer: [u8; 32], amount: Fixed) -> Node<SwapState> {
    let policy = EpochPolicy {
        intents_per_epoch: 10_000,
        epochs_per_anchor: 10_000,
        max_seconds_per_epoch: 0,
        max_seconds_per_anchor: 0,
    };
    let mut n: Node<SwapState> = Node::new(CHAIN, Params::v1(), policy, Economics::flat(10_000));
    let observed = n.state().backing_of(XZEC).add(amount).unwrap();
    n.submit_operator(Intent::AttestVaultBalance { asset: XZEC, observed }, 0);
    let d = Intent::next_deposit(n.state(), buyer, XZEC, amount, [0u8; 32]);
    n.submit_operator(d, 0);
    let epoch = n.state().epoch;
    n.submit_operator(Intent::Checkpoint, 0);
    n.submit_operator(Intent::ConfirmAnchor { epoch }, 0);
    n
}

fn held(rpc: &RpcNode, k: &SigningKey) -> Fixed {
    rpc.account(k)
        .expect("read")
        .map(|r| {
            r.spendable
                .iter()
                .find(|(a, _)| *a == XZEC)
                .map(|(_, v)| *v)
                .unwrap_or(Fixed::ZERO)
        })
        .unwrap_or(Fixed::ZERO)
}

/// Relay a frame the holder signed, the way the sale service does.
fn relay(rpc: &RpcNode, k: &SigningKey, intent: Intent) {
    let epoch = rpc.status().expect("status").epoch;
    let frame = frame_submission(k, CHAIN, epoch, &intent);
    rpc.submit_raw(&frame).expect("the chain accepts it");
}

#[test]
fn an_entry_funds_the_pot_and_a_redemption_is_paid_from_it() {
    let treasury = key(0x01);
    let buyer = key(0x02);
    let (t, b) = (account_of(&treasury), account_of(&buyer));

    // The published split, read from the terms rather than computed here.
    let terms = SaleTerms {
        policy_digest: [0; 32],
        collection_id: [0; 32],
        manifest_root: [0; 32],
        treasury: t,
        network: 0,
        entry_price_zatoshi: 1_000_000,
        claim_price_zatoshi: 500_000,
        open_mint_price_zatoshi: 1_330_000,
        min_confirmations: 6,
        claim_window_blocks: 16_128,
        treasury_bps: 1_000,
    };
    let entry = Fixed(terms.entry_price_zatoshi as i128 * ZAT);
    let (pool_zat, treasury_zat) = terms.split(terms.entry_price_zatoshi);
    assert_eq!((pool_zat, treasury_zat), (900_000, 100_000), "the published 90/10");
    let pool_share = Fixed(pool_zat as i128 * ZAT);

    // Expectations stated independently of the amount the action uses. Deriving
    // both from `pool_share` would only prove that what you fund is what lands
    // — it would not notice the sale funding 100% of the entry, or 50%.
    let expect_pool = Fixed(900_000 * ZAT);
    let expect_treasury_keeps = Fixed(100_000 * ZAT);
    assert_eq!(expect_pool.add(expect_treasury_keeps).unwrap(), entry, "the two halves are the entry");

    let addr = start(chain_with_a_credited_buyer(b, entry));
    let rpc = RpcNode::new(&addr.to_string(), CHAIN);

    // The creator opens the collection and reaches the minting phase.
    let symbol = swapvm::state::symbol(b"CAVEZ");
    relay(&rpc, &treasury, Intent::CreateCollection {
        creator: t,
        symbol,
        cap: 4_444,
        fee_bps: 0,
    });
    let collection = zyn_vm::collection_address(zyn_vm::derive::ADDRESS_SCOPE_V1, &t, b"CAVEZ");
    relay(&rpc, &treasury, Intent::AdvanceCollection { creator: t, collection, to: 1 });

    assert_eq!(held(&rpc, &buyer), entry, "the deposit was credited");
    assert_eq!(held(&rpc, &treasury), Fixed::ZERO);

    // 1. The buyer signs their entry; the gateway relays the bytes untouched.
    relay(&rpc, &buyer, Intent::Transfer { from: b, to: t, asset: XZEC, amount: entry });
    assert_eq!(held(&rpc, &buyer), Fixed::ZERO, "the buyer actually paid");
    assert_eq!(held(&rpc, &treasury), entry, "and the treasury received it");

    // 2. The treasury forwards the pool's share.
    relay(&rpc, &treasury, Intent::FundCollection { from: t, collection, amount: pool_share });
    let view = rpc.collection(collection).expect("collection").expect("exists");
    assert_eq!(view.pool, expect_pool, "90% of the entry reached the pool");
    assert_eq!(
        held(&rpc, &treasury),
        expect_treasury_keeps,
        "and the treasury kept exactly 10%"
    );

    // 3. The item is minted to the account that paid.
    relay(&rpc, &treasury, Intent::MintCollectionItem {
        creator: t,
        collection,
        serial: 0,
        to: b,
        symbol: swapvm::state::symbol(b"CAVEZ0001"),
        content: [0xc0; 32],
    });
    let view = rpc.collection(collection).expect("collection").expect("exists");
    assert_eq!(view.outstanding, 1);
    // Before the market opens the quotient is not a floor, and says so.
    assert_eq!(view.redeem_price, Fixed::ZERO, "no floor before the collection is live");

    // 4. The market opens.
    relay(&rpc, &treasury, Intent::AdvanceCollection { creator: t, collection, to: 2 });
    relay(&rpc, &treasury, Intent::AdvanceCollection { creator: t, collection, to: 3 });
    let view = rpc.collection(collection).expect("collection").expect("exists");
    assert_eq!(view.redeem_price, expect_pool, "one item outstanding, so the floor is the whole pool");

    // 5. The holder hands the item back and is paid out of the pot.
    let item = zyn_vm::item_address(zyn_vm::derive::ADDRESS_SCOPE_V1, &collection, 0);
    relay(&rpc, &buyer, Intent::RedeemCollectionItem { holder: b, asset: item });

    assert_eq!(
        held(&rpc, &buyer),
        expect_pool,
        "the buyer paid the entry and redeemed for 90% of it — the 10% is what the sale kept"
    );
    let view = rpc.collection(collection).expect("collection").expect("exists");
    assert_eq!(view.outstanding, 0, "the item is gone");
    assert_eq!(view.pool, Fixed::ZERO, "and the pool went with it");
}
