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
        // Atomic routes (§167): a coin's curve, a pool, another coin's curve.
        // A collection's DAO (§172): its program, a pass hash, a payment's
        // claim source, and the three member intents.
        ("program_dao", hex(&program::dao().0)),
        ("dao_pass", hex(&objectvm::dao::pass_of(b"cave-pass-token"))),
        ("dao_payment_source", hex(&objectvm::dao::payment_source(&TOKEN, 7))),
        (
            "member_pass",
            enc(&Op::MemberPass { holder: OWNER, program: program::dao(), collection: TOKEN, item: [0xc1; 32], pass: objectvm::dao::pass_of(b"cave-pass-token") }),
        ),
        (
            "propose",
            enc(&Op::Propose {
                proposer: OWNER,
                program: program::dao(),
                collection: TOKEN,
                item: [0xc1; 32],
                text: [0x7a; 32],
                pay: Some(objectvm::dao::Payment { asset: ZEC, amount: Fixed::whole(3), to: [0xB0; 32] }),
            }),
        ),
        (
            "propose_no_pay",
            enc(&Op::Propose { proposer: OWNER, program: program::dao(), collection: TOKEN, item: [0xc1; 32], text: [0x7a; 32], pay: None }),
        ),
        // The Zyn track (§176): a member's Zyn proposal.
        (
            "propose_zyn",
            enc(&Op::ProposeZyn { proposer: OWNER, program: program::dao(), collection: TOKEN, item: [0xc1; 32], text: [0x7a; 32] }),
        ),
        (
            "vote",
            enc(&Op::Vote { voter: OWNER, program: program::dao(), collection: TOKEN, proposal: 7, items: vec![[0xc1; 32], [0xc2; 32]], choice: objectvm::dao::YES }),
        ),
        // A gated sale's list (§168): an account's blinded key, and prices.
        ("sale_allow_key", hex(&objectvm::sale::allow_key(&TOKEN, &OWNER))),
        ("sale_gated_total_10_2", objectvm::sale::gated_total(Fixed(10_000_000_000_000_000), 10, 10, 2).unwrap().to_string()),
        ("program_route", hex(&program::route().0)),
        ("route_id", hex(&objectvm::route::route_id(&OWNER, &NONCE))),
        ("route_leg_1", hex(&objectvm::route::leg_id(&objectvm::route::route_id(&OWNER, &NONCE), 1))),
        (
            "route",
            enc(&Op::Route {
                trader: OWNER,
                program: program::route(),
                hops: vec![
                    objectvm::route::Hop::CurveSell { token: TOKEN },
                    objectvm::route::Hop::Pool { pool, sell_a: false },
                    objectvm::route::Hop::CurveBuy { token: [0xe1; 32] },
                ],
                amount_in: Fixed::whole(125_000),
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
                inputs: input.clone(),
                change: vec![change()],
                nonce: NONCE,
            }),
        ),
        // Bonding curves (DECISIONS §141).
        ("program_curve_create", hex(&program::curve_create().0)),
        ("program_curve_trade", hex(&program::curve_trade().0)),
        ("curve_source", hex(&objectvm::curve::curve_source(&OWNER, &NONCE))),
        ("curve_order_id", hex(&objectvm::curve::curve_order_id(&OWNER, &NONCE))),
        ("curve_token", hex(&market::token_of(&OWNER, &NONCE))),
        ("curve_cost_to_graduation", objectvm::curve::cost(0, objectvm::curve::GRADUATE_AT).to_string()),
        ("curve_cost_1000_50", objectvm::curve::cost(1_000, 50).to_string()),
        // Sized for another coin (§166): ETH at 23 ETH, BOLD at 63,000 BOLD.
        ("curve_unit_23", objectvm::curve::QuoteTerms::unit_for(zyn_vm::Fixed::whole(23)).to_string()),
        ("curve_unit_63000", objectvm::curve::QuoteTerms::unit_for(zyn_vm::Fixed::whole(63_000)).to_string()),
        (
            "curve_cost_1000_50_at_23",
            objectvm::curve::cost_at(1_000, 50, objectvm::curve::QuoteTerms::unit_for(zyn_vm::Fixed::whole(23))).to_string(),
        ),
        ("curve_graduation_mc_units", objectvm::curve::GRADUATION_MC_UNITS.to_string()),
        (
            "create_curve",
            enc(&Op::CreateCurve {
                creator: OWNER,
                program: program::curve_create(),
                nonce: NONCE,
                quote: ZEC,
                meta: [0x4d; 32],
                inputs: input.clone(),
                change: vec![change()],
            }),
        ),
        (
            "curve_trade",
            enc(&Op::CurveTrade {
                trader: OWNER,
                program: program::curve_trade(),
                token: TOKEN,
                buy: true,
                amount_in: Fixed::whole(1),
                min_out: objectvm::curve::LOT,
                inputs: input.clone(),
                change: vec![change()],
                nonce: NONCE,
            }),
        ),
    ]
    .into_iter()
    .chain(sale_vectors(&input))
    .chain(bridge_vectors())
    .chain(lend_vectors(&input))
    .collect()
}

/// Zyn Lend: its programs, derived ids and the seven intents a holder signs.
fn lend_vectors(input: &[([u8; 32], [u8; 32])]) -> Vec<(&'static str, String)> {
    use objectvm::lend::{leftover_source, lend_asset, lend_source, lot_source};
    let enc = |op: &Op| hex(&ObjectVm::encode_intent(op));
    let position = lend_source(&OWNER, &NONCE);
    vec![
        ("program_lend_supply", hex(&program::lend_supply().0)),
        ("program_lend_withdraw", hex(&program::lend_withdraw().0)),
        ("program_lend_open", hex(&program::lend_open().0)),
        ("program_lend_repay", hex(&program::lend_repay().0)),
        ("program_lend_collateral", hex(&program::lend_collateral().0)),
        ("program_lend_bid", hex(&program::lend_bid().0)),
        ("lend_asset", hex(&lend_asset(&ZEC))),
        ("lend_source", hex(&position)),
        ("lot_source", hex(&lot_source(&position))),
        ("leftover_source", hex(&leftover_source(&position))),
        (
            "lend_supply",
            enc(&Op::LendSupply { lender: OWNER, program: program::lend_supply(), asset: ZEC, amount: Fixed::whole(3), nonce: NONCE, inputs: input.to_vec(), change: vec![change()] }),
        ),
        (
            "lend_withdraw",
            enc(&Op::LendWithdraw { lender: OWNER, program: program::lend_withdraw(), asset: ZEC, shares: Fixed::whole(3), nonce: NONCE, inputs: input.to_vec(), change: vec![change()] }),
        ),
        (
            "lend_open",
            enc(&Op::LendOpen {
                borrower: OWNER,
                program: program::lend_open(),
                collateral_asset: ZEC,
                collateral: Fixed::whole(5),
                borrow_asset: TOKEN,
                amount: Fixed::whole(1),
                nonce: NONCE,
                inputs: input.to_vec(),
                change: vec![change()],
            }),
        ),
        (
            "lend_repay",
            enc(&Op::LendRepay { borrower: OWNER, program: program::lend_repay(), position, amount: Fixed::whole(1), nonce: NONCE, inputs: input.to_vec(), change: vec![change()] }),
        ),
        (
            "lend_add_collateral",
            enc(&Op::LendAddCollateral { borrower: OWNER, program: program::lend_collateral(), position, amount: Fixed::whole(1), inputs: input.to_vec(), change: vec![change()] }),
        ),
        (
            "lend_remove_collateral",
            enc(&Op::LendRemoveCollateral { borrower: OWNER, program: program::lend_collateral(), position, amount: Fixed::whole(1), nonce: NONCE }),
        ),
        (
            "lend_bid",
            enc(&Op::LendBid { bidder: OWNER, program: program::lend_bid(), lot: lot_source(&position), pay: Fixed::whole(1), nonce: NONCE, inputs: input.to_vec(), change: vec![change()] }),
        ),
    ]
}

/// Collection Z's lottery (DECISIONS §144): ids, leaves, a proof and the
/// three intents a holder signs.
fn sale_vectors(input: &[([u8; 32], [u8; 32])]) -> Vec<(&'static str, String)> {
    use objectvm::sale::{draw_leaf, open_leaf, ticket_id, treasury_source, voucher_commitment, voucher_ticket_id};
    use zyn_vm::commit::{merkle_proof, merkle_root};
    let enc = |op: &Op| hex(&ObjectVm::encode_intent(op));
    let ticket = ticket_id(&OWNER, &NONCE, 1);
    let leaves = vec![draw_leaf(&[0x01; 32], &[4]), draw_leaf(&ticket, &[7, 1]), draw_leaf(&[0x03; 32], &[0, 2, 9])];
    let proof = merkle_proof(&leaves, 1);
    let proof_hex: Vec<String> = proof.iter().map(|p| format!("{}{}", hex(&p.sibling), if p.node_is_right { "1" } else { "0" })).collect();
    let item = |serial: u32| {
        let mut o = change();
        o.note.asset = market::item_asset(&TOKEN, serial);
        o.note.amount = market::ITEM_UNIT;
        o
    };
    vec![
        ("program_sale_tickets", hex(&program::sale_tickets().0)),
        ("program_sale_claim", hex(&program::sale_claim().0)),
        ("program_sale_mint", hex(&program::sale_mint().0)),
        ("ticket_id", hex(&ticket)),
        ("treasury_source", hex(&treasury_source(&TOKEN, &ticket))),
        ("draw_leaf", hex(&draw_leaf(&ticket, &[7, 1]))),
        ("open_leaf", hex(&open_leaf(3, 11))),
        ("draw_root", hex(&merkle_root(&leaves))),
        ("draw_proof", proof_hex.join(",")),
        (
            "buy_tickets",
            enc(&Op::BuyTickets {
                buyer: OWNER,
                program: program::sale_tickets(),
                collection: TOKEN,
                count: 3,
                nonce: NONCE,
                inputs: input.to_vec(),
                change: vec![change()],
            }),
        ),
        (
            "claim_items",
            enc(&Op::ClaimItems {
                buyer: OWNER,
                program: program::sale_claim(),
                collection: TOKEN,
                nonce: NONCE,
                k: 1,
                serials: vec![7, 1],
                proof: proof.clone(),
                items: vec![item(7), item(1)],
                inputs: input.to_vec(),
                change: vec![change()],
            }),
        ),
        ("voucher_commitment", hex(&voucher_commitment(&OWNER, &NONCE))),
        ("voucher_ticket_id", hex(&voucher_ticket_id(&voucher_commitment(&OWNER, &NONCE), 2))),
        (
            "claim_voucher_items",
            enc(&Op::ClaimVoucherItems {
                buyer: OWNER,
                program: program::sale_claim(),
                collection: TOKEN,
                secret: NONCE,
                k: 2,
                serials: vec![7, 1],
                proof: proof.clone(),
                items: vec![item(7), item(1)],
                inputs: input.to_vec(),
                change: vec![change()],
            }),
        ),
        (
            "mint_open",
            enc(&Op::MintOpen {
                buyer: OWNER,
                program: program::sale_mint(),
                collection: TOKEN,
                serial: 11,
                proof,
                item: item(11),
                nonce: NONCE,
                inputs: input.to_vec(),
                change: vec![change()],
            }),
        ),
    ]
}

/// Bridged assets beside ZEC (§146): ids from public facts, the deposit
/// commitment and its key, the claim, and an exit's destination commitment.
fn bridge_vectors() -> Vec<(&'static str, String)> {
    use objectvm::bridge::{asset_of, deposit_commitment, deposit_key};
    let sol = asset_of(b"solana-devnet", b"");
    let com = deposit_commitment(&OWNER, &NONCE);
    let mut out = change();
    out.note.asset = sol;
    vec![
        ("bridged_sol_devnet", hex(&sol)),
        ("bridged_eth_sepolia", hex(&asset_of(b"eip155:11155111", b""))),
        ("deposit_commitment", hex(&com)),
        ("deposit_key", hex(&deposit_key(&com, &sol))),
        ("exit_commitment_evm", hex(&zyn_vm::eip712::keccak(&[&[0x11; 20][..], &NONCE[..]]))),
        (
            "claim_deposit",
            hex(&ObjectVm::encode_intent(&Op::ClaimDeposit { owner: OWNER, program: program::claim(), secret: NONCE, asset: sol, output: out })),
        ),
    ]
}

