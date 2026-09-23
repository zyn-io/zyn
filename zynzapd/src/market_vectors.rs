//! The market's wire formats, as vectors the browser is held to.
//!
//! One function builds them; `zyn-object-vectors` prints them into the shared
//! fixture and `object_vectors_are_pinned` checks this function still produces
//! what was printed. The browser's test checks its own encoders against the
//! same fixture, so a change to either side fails somewhere.

use objectvm::market;
use objectvm::{ObjectVm, Op, Output};
use zyn_vm::note_enc::{seal, viewing_public, StaticSecret};
use zyn_vm::object::Note;
use zyn_vm::spec::MicrochainVm;
use zyn_vm::{program, Fixed};

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

pub const ZEC: [u8; 32] = [0x0e; 32];
pub const TOKEN: [u8; 32] = [0xe0; 32];
pub const OWNER: [u8; 32] = [0xA1; 32];
pub const NONCE: [u8; 32] = [0x6e; 32];

/// The change note every vector carries, sealed with fixed keys.
pub fn change() -> Output {
    let note = Note {
        asset: ZEC,
        amount: Fixed::whole(2),
        owner: OWNER,
        nonce: [0x77; 32],
    };
    let mut plain = Vec::with_capacity(80);
    plain.extend_from_slice(&note.asset);
    plain.extend_from_slice(&note.amount.0.to_be_bytes());
    plain.extend_from_slice(&note.nonce);
    let enc = seal(
        &viewing_public(&StaticSecret::from([0xB0u8; 32])),
        StaticSecret::from([0x55u8; 32]),
        &plain,
    )
    .expect("seals");
    Output { note, enc }
}

/// `(name, hex)` for every program id, derived id and encoded intent.
pub fn vectors() -> Vec<(&'static str, String)> {
    let input = vec![([0xC1; 32], [0xD1; 32]), ([0xC2; 32], [0xD2; 32])];
    let pool = market::pool_id(&ZEC, &TOKEN);
    let order = market::order_id(&OWNER, &NONCE);
    let enc = |op: &Op| hex(&ObjectVm::encode_intent(op));
    vec![
        ("program_amm_swap", hex(&program::amm_swap().0)),
        ("program_add_liquidity", hex(&program::add_liquidity().0)),
        ("program_remove_liquidity", hex(&program::remove_liquidity().0)),
        ("program_claim", hex(&program::claim().0)),
        ("program_launch_create", hex(&program::launch_create().0)),
        ("program_launch_commit", hex(&program::launch_commit().0)),
        ("program_withdraw", hex(&program::withdraw().0)),
        ("pool_id", hex(&pool)),
        ("lp_asset", hex(&market::lp_asset(&pool))),
        ("token_of", hex(&market::token_of(&OWNER, &NONCE))),
        ("launch_source", hex(&market::launch_source(&OWNER, &NONCE))),
        ("order_id", hex(&order)),
        ("commit_id", hex(&market::commit_id(&OWNER, &NONCE))),
        ("liquidity_id", hex(&market::liquidity_id(&OWNER, &NONCE))),
        ("claim_id_order_0", hex(&market::claim_id(&order, 0))),
        (
            "create_pool",
            enc(&Op::CreatePool {
                provider: OWNER,
                program: program::add_liquidity(),
                asset_a: ZEC,
                asset_b: TOKEN,
                amount_a: Fixed::whole(10),
                amount_b: Fixed::whole(40),
                fee_bps: 30,
                inputs: input.clone(),
                change: vec![change()],
                nonce: NONCE,
            }),
        ),
        (
            "add_liquidity",
            enc(&Op::AddLiquidity {
                provider: OWNER,
                program: program::add_liquidity(),
                pool,
                amount_a: Fixed::whole(1),
                amount_b: Fixed::whole(4),
                min_shares: Fixed(12345),
                inputs: input.clone(),
                change: vec![change()],
                nonce: NONCE,
            }),
        ),
        (
            "remove_liquidity",
            enc(&Op::RemoveLiquidity {
                provider: OWNER,
                program: program::remove_liquidity(),
                pool,
                shares: Fixed::whole(3),
                min_a: Fixed(1),
                min_b: Fixed(2),
                inputs: input.clone(),
                change: vec![change()],
                nonce: NONCE,
            }),
        ),
        (
            "swap",
            enc(&Op::Swap {
                trader: OWNER,
                program: program::amm_swap(),
                pool,
                sell_a: true,
                amount_in: Fixed::whole(5),
                min_out: Fixed(4_900_000_000_000_000_000),
                inputs: input.clone(),
                change: vec![change()],
                nonce: NONCE,
            }),
        ),
        (
            "claim",
            enc(&Op::Claim {
                owner: OWNER,
                program: program::claim(),
                claims: vec![market::claim_id(&order, 0), market::claim_id(&order, 1)],
                outputs: vec![change()],
            }),
        ),
        (
            "create_launch",
            enc(&Op::CreateLaunch {
                creator: OWNER,
                program: program::launch_create(),
                nonce: NONCE,
                quote: ZEC,
                total_supply: Fixed::whole(1_000_000),
                sale_supply: Fixed::whole(600_000),
                pool_supply: Fixed::whole(200_000),
                soft_cap: Fixed::whole(10),
                hard_cap: Fixed::whole(100),
                pool_bps: 5000,
                fee_bps: 30,
                closes_at_epoch: 4321,
                meta: [0x4d; 32],
            }),
        ),
        ("set_fees", enc(&Op::SetFees { program: program::configure_fees(), bridge_bps: 50 })),
        (
            "register_collection",
            enc(&Op::RegisterCollection {
                program: program::register_collection(),
                asset: TOKEN,
                creator: OWNER,
                quote: ZEC,
                fee_bps: 100,
                cap: 4444,
            }),
        ),
        (
            "redeem",
            enc(&Op::Redeem { holder: OWNER, program: program::redeem_item(), asset: TOKEN, inputs: input.clone() }),
        ),
        (
            "list",
            enc(&Op::List {
                seller: OWNER,
                program: program::place_offer(),
                item: [0xC1; 32],
                item_nullifier: [0xD1; 32],
                price_asset: ZEC,
                price_amount: Fixed::whole(2),
                expires_at_epoch: 99_999,
                nonce: NONCE,
            }),
        ),
        (
            "cancel_offer",
            enc(&Op::CancelOffer { seller: OWNER, program: program::cancel_offer(), item: [0xC1; 32] }),
        ),
        (
            "take_offer",
            enc(&Op::TakeOffer {
                buyer: OWNER,
                program: program::take_offer(),
                item: [0xC1; 32],
                item_nullifier: [0xD1; 32],
                payment: input.clone(),
                outputs: vec![change()],
            }),
        ),
        ("item_asset_serial_7", hex(&market::item_asset(&TOKEN, 7))),
        ("listing_source", hex(&market::listing_source(&OWNER, &NONCE))),
        ("sale_fee_source", hex(&market::sale_fee_source(&[0xC1; 32]))),
        ("redemption_source", hex(&market::redemption_source(&[0xC1; 32]))),
        (
            "withdraw",
            enc(&Op::Withdraw {
                holder: OWNER,
                program: program::withdraw(),
                inputs: input.clone(),
                asset: ZEC,
                amount: Fixed::whole(3),
                destination: [0xDE; 32],
                outputs: vec![change()],
            }),
        ),
        (
            "commit",
            enc(&Op::Commit {
                buyer: OWNER,
                program: program::launch_commit(),
                launch: TOKEN,
                amount: Fixed::whole(7),
                inputs: input,
                change: vec![change()],
                nonce: NONCE,
            }),
        ),
    ]
}
