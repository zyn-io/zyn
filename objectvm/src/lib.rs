//! `ObjectVm` — the object ledger as a complete Zyn application.
//!
//! Where [`TallyVm`](../../zyn/tests/tally) is the smallest thing that
//! satisfies the specification, this is the smallest thing that satisfies it
//! *with value held as objects instead of balances*. It exists so the model in
//! DECISIONS §130 can be run end to end — minted, spent, conserved, sealed —
//! without touching swapvm, which keeps chains 11 and 26460 on the account
//! ledger while this one is free to break.
//!
//! # What an account is here
//!
//! Not a balance sheet. An account is an **authority** — an id and a spend
//! counter — and nothing about what it holds. Value lives in objects:
//!
//! ```text
//!   account 7f3a…  →  { nonce: 9 }                     who may spend
//!   commitments    →  [ C₁, C₂, C₃, … ]                what exists
//!   nullifiers     →  { N₄, N₇ }                       what has been spent
//! ```
//!
//! Section 1 stays the accounts section because the specification pins it
//! there and the exit hatch opens it. It simply stops carrying value.
//!
//! # What is published and what is not
//!
//! [`MicrochainVm::sections`] commits the commitment root and the nullifier
//! root. It does **not** commit amounts or owners. A holder of the published
//! sections learns how many objects exist and how many have been spent, and
//! nothing else — in particular, nothing links a nullifier to the commitment
//! it retired.
//!
//! Amounts *are* in [`MicrochainVm::encode`], because the sequencer and every
//! replica must check conservation and cannot sum what they cannot read. That
//! is tier 2 exactly as §130 defines it: private from the public, visible to
//! the operator. Hiding amounts from the operator too is tier 3, and it is
//! where [`Live`] stops holding a [`Fixed`] and starts holding a commitment.
//!
//! # Conservation
//!
//! §130.6 worried that commitments break `conserved()`. They do not break it
//! *here*: the sum runs over live note amounts instead of account balances,
//! which is the same check over a different container. It breaks at tier 3,
//! when there is no amount left to sum — which is the argument for doing the
//! container change and the hiding change as two steps rather than one.

use std::collections::{BTreeMap, BTreeSet};
use std::vec as alloc_vec;

pub mod bridge;
pub mod lend;
/// The lending rules `lend` runs on (zyn-io/zyn-lend), for readers of its state.
pub use zyn_lend;
pub mod curve;
pub mod route;
pub mod sale;
pub mod market;

use zyn_vm::commit::{merkle_root, Encoder, Hash};
use zyn_vm::derive::Address;
use zyn_vm::note_enc::{EncryptedNote, ViewingKey};
use zyn_vm::object::{commitment, Commitments, Note, Nullifiers, ObjectError};
use zyn_vm::program::{self, ProgramId};
use zyn_vm::read::Decoder;
use zyn_vm::spec::{AccountId, MicrochainVm};
use zyn_vm::{Checkpoint, Fixed};

/// The application's committed parameters.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Limits {
    /// Most objects one spend may consume, and most it may create. Bounds the
    /// work a single intent can cost, which **S5** requires be enforced rather
    /// than computed.
    pub max_io: u32,
}

/// What a spend says. The nullifier is supplied rather than derived, because
/// deriving it needs the owner's nullifier key and the VM must never see one:
/// a key in an intent is a key in the journal, and from there anyone could
/// compute the nullifiers of every *other* object that key controls.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Op {
    /// Devnet issuance. On a real chain this is what a confirmed deposit
    /// produces; here it is the faucet that makes the rest testable.
    Mint {
        asset: Address,
        amount: Fixed,
        owner: AccountId,
        nonce: Hash,
    },
    /// Consume objects and create their successors, under a named rule.
    Spend {
        spender: AccountId,
        /// Which law this transaction claims to obey (DECISIONS §131.4).
        /// Today the VM implements the rule itself; later this may name a
        /// program proved elsewhere, and the transaction will not change
        /// shape when it does.
        program: ProgramId,
        /// `(commitment, nullifier)` for each object being consumed.
        inputs: Vec<(Hash, Hash)>,
        outputs: Vec<Output>,
    },
    /// The vault paid an exit. Names the Zcash transaction, so the claim can
    /// be checked against the chain rather than believed.
    SettleExit {
        program: ProgramId,
        /// Index into `exits`, which is append-only, so an index is stable.
        index: u32,
        txid: Hash,
    },
    /// Publish where to pay this account.
    ///
    /// Re-registering replaces the key, because a holder who loses one needs
    /// a way to be paid again and the old key cannot spend anything — it only
    /// ever found notes. What it costs is that anyone able to submit as this
    /// account can redirect *future* discoverability, not any existing note.
    Register {
        account: AccountId,
        program: ProgramId,
        viewing: ViewingKey,
    },
    /// Leave for Zcash: burn objects and record a payout for the vault.
    ///
    /// The mirror of a deposit, and the other rule that changes supply. A
    /// deposit cannot mint more than the vault was paid; a withdrawal cannot
    /// burn more than the holder owned, and the supply falls by exactly what
    /// leaves — otherwise the chain would claim backing it no longer has.
    Withdraw {
        holder: AccountId,
        program: ProgramId,
        /// Objects consumed to fund it.
        inputs: Vec<(Hash, Hash)>,
        /// What leaves the system.
        asset: Address,
        amount: Fixed,
        /// `H(zcash address ‖ salt)`.
        destination: Hash,
        /// Change back to the holder.
        outputs: Vec<Output>,
    },
    /// ZEC confirmed in the vault becomes an object for its depositor.
    ///
    /// The one rule that creates value rather than moving it, and the only
    /// one carrying an `external_ref`: a backing signer decrypts that Zcash
    /// transaction, sums what it actually paid the vault, and refuses if it
    /// is less than `amount` (§130.13). That check is why the amount is in
    /// the clear here and why tier 3 cannot simply hide it.
    Deposit {
        program: ProgramId,
        asset: Address,
        amount: Fixed,
        owner: AccountId,
        nonce: Hash,
        /// How the depositor finds the object we just made for them. Without
        /// it the note exists and is unspendable (§133.6).
        enc: EncryptedNote,
        /// The Zcash txid this is backed by.
        external_ref: Hash,
    },
    /// Encumber an object against a price. It does not move.
    PlaceOffer {
        seller: AccountId,
        program: ProgramId,
        item: Hash,
        price_asset: Address,
        price_amount: Fixed,
        expires_at_epoch: u64,
    },
    /// Lift an encumbrance. Transfers nothing.
    CancelOffer {
        seller: AccountId,
        program: ProgramId,
        item: Hash,
    },
    /// Pay the price and take the object, atomically (§132.2).
    TakeOffer {
        buyer: AccountId,
        program: ProgramId,
        item: Hash,
        /// The item's nullifier. Supplied by the taker, who cannot compute the
        /// seller's — so a listing publishes it, which is the disclosure
        /// §132.5 accepts and scopes.
        item_nullifier: Hash,
        /// The buyer's own objects, funding the price.
        payment: Vec<(Hash, Hash)>,
        /// Price to the seller, item to the buyer, change to the buyer.
        outputs: Vec<Output>,
    },
    /// Open a pool for a pair of assets with its first liquidity.
    CreatePool {
        provider: AccountId,
        program: ProgramId,
        /// The lower asset in byte order; `asset_b` the higher.
        asset_a: Address,
        asset_b: Address,
        amount_a: Fixed,
        amount_b: Fixed,
        fee_bps: u32,
        inputs: Vec<(Hash, Hash)>,
        change: Vec<Output>,
        /// The provider's secret; their claims derive from it.
        nonce: Hash,
    },
    /// Add to a pool at its current ratio. Shares and any excess come back
    /// as claims.
    AddLiquidity {
        provider: AccountId,
        program: ProgramId,
        pool: Hash,
        amount_a: Fixed,
        amount_b: Fixed,
        min_shares: Fixed,
        inputs: Vec<(Hash, Hash)>,
        change: Vec<Output>,
        nonce: Hash,
    },
    /// Burn shares for their part of both reserves, as claims.
    RemoveLiquidity {
        provider: AccountId,
        program: ProgramId,
        pool: Hash,
        shares: Fixed,
        min_a: Fixed,
        min_b: Fixed,
        inputs: Vec<(Hash, Hash)>,
        change: Vec<Output>,
        nonce: Hash,
    },
    /// Sell into a pool's batch. Escrowed now, priced at the seal.
    Swap {
        trader: AccountId,
        program: ProgramId,
        pool: Hash,
        sell_a: bool,
        amount_in: Fixed,
        min_out: Fixed,
        inputs: Vec<(Hash, Hash)>,
        change: Vec<Output>,
        nonce: Hash,
    },
    /// Turn claims into objects the owner encrypted for themselves.
    Claim {
        owner: AccountId,
        program: ProgramId,
        claims: Vec<Hash>,
        outputs: Vec<Output>,
    },
    /// Open a token sale.
    CreateLaunch {
        creator: AccountId,
        program: ProgramId,
        nonce: Hash,
        quote: Address,
        total_supply: Fixed,
        sale_supply: Fixed,
        pool_supply: Fixed,
        soft_cap: Fixed,
        hard_cap: Fixed,
        pool_bps: u32,
        fee_bps: u32,
        closes_at_epoch: u64,
        meta: Hash,
    },
    /// Commit to a sale.
    Commit {
        buyer: AccountId,
        program: ProgramId,
        launch: Address,
        amount: Fixed,
        inputs: Vec<(Hash, Hash)>,
        change: Vec<Output>,
        nonce: Hash,
    },
    /// Set what the chain charges. Operator only; nothing is charged before it.
    SetFees {
        program: ProgramId,
        bridge_bps: u32,
    },
    /// Give a collection resale terms and a redemption floor. Operator only,
    /// once per collection: holders price items against these terms, so they
    /// do not move under them.
    RegisterCollection {
        program: ProgramId,
        asset: Address,
        creator: AccountId,
        quote: Address,
        fee_bps: u32,
        /// The most items that will ever exist.
        cap: u32,
    },
    /// A deposit with the bridge fee taken from it: the vault received
    /// `gross`, the depositor's note is `amount`, the difference goes to the
    /// ZYN pot. Issued supply rises by `gross`, because the vault holds it.
    DepositWithFee {
        program: ProgramId,
        asset: Address,
        gross: Fixed,
        amount: Fixed,
        owner: AccountId,
        nonce: Hash,
        enc: EncryptedNote,
        external_ref: Hash,
    },
    /// List an item for sale, paid by claim. The listing publishes the
    /// item's nullifier, so a buyer can retire it, and names no seller: the
    /// proceeds go to a claim only the seller's nonce finds.
    List {
        seller: AccountId,
        program: ProgramId,
        item: Hash,
        item_nullifier: Hash,
        price_asset: Address,
        price_amount: Fixed,
        expires_at_epoch: u64,
        nonce: Hash,
    },
    /// Mint one item of a registered collection to its buyer. Operator only.
    MintItem {
        program: ProgramId,
        collection: Address,
        serial: u32,
        owner: AccountId,
        nonce: Hash,
        enc: EncryptedNote,
    },
    /// Burn items for their share of the collection's floor.
    Redeem {
        holder: AccountId,
        program: ProgramId,
        asset: Address,
        inputs: Vec<(Hash, Hash)>,
    },
    /// Make a bonding-curve token. The creator pays the bond, if the
    /// operator set one, and holds nothing else: every token is bought.
    CreateCurve {
        creator: AccountId,
        program: ProgramId,
        nonce: Hash,
        quote: Address,
        meta: Hash,
        inputs: Vec<(Hash, Hash)>,
        change: Vec<Output>,
    },
    /// Buy from or sell to a curve; escrowed, settled at the seal.
    CurveTrade {
        trader: AccountId,
        program: ProgramId,
        token: Address,
        buy: bool,
        amount_in: Fixed,
        min_out: Fixed,
        inputs: Vec<(Hash, Hash)>,
        change: Vec<Output>,
        nonce: Hash,
    },
    /// Set what curves charge. Operator only; nothing is charged before it.
    SetCurveTerms {
        program: ProgramId,
        fee_bps: u32,
        bond: Fixed,
    },
    /// One order through up to four venues — a coin's curve, pools, another
    /// coin's curve — settled in one seal, all or nothing (§167).
    Route {
        trader: AccountId,
        program: ProgramId,
        hops: Vec<route::Hop>,
        amount_in: Fixed,
        min_out: Fixed,
        inputs: Vec<(Hash, Hash)>,
        change: Vec<Output>,
        nonce: Hash,
    },
    /// Gate a sale to a list (§168): only listed accounts buy tickets or
    /// claim burn tickets; each pays the ticket price for its first `base`
    /// tickets and one more ticket price for each after. Operator only,
    /// before the sale's first ticket.
    GateSale {
        program: ProgramId,
        collection: Address,
        base: u32,
    },
    /// Put accounts on a gated sale's list, by `sale::allow_key`. Operator only.
    AllowBuyers {
        program: ProgramId,
        collection: Address,
        keys: Vec<Hash>,
    },
    /// Name an asset coins launch in, sized to graduate at `graduation_mc`
    /// of it fully diluted, with `bond` of it to create one (§166). A zero
    /// `graduation_mc` stops new coins in it. Operator only.
    SetCurveQuote {
        program: ProgramId,
        quote: Address,
        graduation_mc: Fixed,
        bond: Fixed,
    },
    /// Open a collection's primary sale at these prices (§144). Operator only.
    SetSale {
        program: ProgramId,
        collection: Address,
        ticket_price: Fixed,
        claim_price: Fixed,
        open_price: Fixed,
        treasury_bps: u32,
    },
    /// Buy `count` lottery tickets in an open sale.
    BuyTickets {
        buyer: AccountId,
        program: ProgramId,
        collection: Address,
        count: u32,
        nonce: Hash,
        inputs: Vec<(Hash, Hash)>,
        change: Vec<Output>,
    },
    /// Close a sale at the Zcash block it closed at. Operator only.
    CloseSale {
        program: ProgramId,
        collection: Address,
        zcash_height: u32,
        zcash_hash: Hash,
    },
    /// Post the draw's result root. Operator only.
    PostDraw {
        program: ProgramId,
        collection: Address,
        root: Hash,
    },
    /// Claim what ticket `(nonce, k)` drew: proved against the posted root,
    /// paid at the claim price, minted to the claimant.
    ClaimItems {
        buyer: AccountId,
        program: ProgramId,
        collection: Address,
        nonce: Hash,
        k: u32,
        serials: Vec<u32>,
        proof: Vec<zyn_vm::commit::ProofStep>,
        items: Vec<Output>,
        inputs: Vec<(Hash, Hash)>,
        change: Vec<Output>,
    },
    /// End claims and post the order the rest sell in. Operator only.
    PostOpenMint {
        program: ProgramId,
        collection: Address,
        root: Hash,
        count: u32,
    },
    /// Mint the next item of the open-mint order at the open price.
    MintOpen {
        buyer: AccountId,
        program: ProgramId,
        collection: Address,
        serial: u32,
        proof: Vec<zyn_vm::commit::ProofStep>,
        item: Output,
        nonce: Hash,
        inputs: Vec<(Hash, Hash)>,
        change: Vec<Output>,
    },
    /// How many burn tickets a sale takes (§145). Operator only.
    SetVoucherCap {
        program: ProgramId,
        collection: Address,
        cap: u32,
    },
    /// One burn on another chain, recorded once, as `count` tickets under
    /// ids from the burner's `commitment` (§145). Operator only.
    PostVoucher {
        program: ProgramId,
        collection: Address,
        burn: Hash,
        commitment: Hash,
        count: u32,
    },
    /// Claim what burn ticket `k` drew, revealing the secret behind the
    /// burn's commitment to this account (§145).
    ClaimVoucherItems {
        buyer: AccountId,
        program: ProgramId,
        collection: Address,
        secret: Hash,
        k: u32,
        serials: Vec<u32>,
        proof: Vec<zyn_vm::commit::ProofStep>,
        items: Vec<Output>,
        inputs: Vec<(Hash, Hash)>,
        change: Vec<Output>,
    },
    /// Register an asset bridged from another chain (§146). Operator only.
    RegisterBridged {
        program: ProgramId,
        network: Vec<u8>,
        origin: Vec<u8>,
        decimals: u8,
    },
    /// One deposit on another chain, recorded once by `external_ref`, waiting
    /// for whoever holds the secret behind `commitment` (§146). Operator only.
    DepositCommitted {
        program: ProgramId,
        asset: Address,
        gross: Fixed,
        external_ref: Hash,
        commitment: Hash,
    },
    /// Take what waits for this account's commitment as one note (§146).
    ClaimDeposit {
        owner: AccountId,
        program: ProgramId,
        secret: Hash,
        asset: Address,
        output: Output,
    },
    /// Open a lending pool for `asset`, or change its collateral risk
    /// (Zyn Lend). Operator only.
    LendList {
        program: ProgramId,
        asset: Address,
        max_ltv_bps: u32,
        liquidation_bps: u32,
        penalty_bps: u32,
    },
    /// Prices lending runs on, and the time they describe: every pool accrues
    /// interest to `at`. Operator only.
    LendPrices {
        program: ProgramId,
        at: u64,
        prices: Vec<(Address, Fixed)>,
    },
    /// Lend `amount` of `asset` for receipt shares, paid as a claim.
    LendSupply {
        lender: AccountId,
        program: ProgramId,
        asset: Address,
        amount: Fixed,
        nonce: Hash,
        inputs: Vec<(Hash, Hash)>,
        change: Vec<Output>,
    },
    /// Burn receipt shares of `asset`'s pool for what they are worth, paid as
    /// a claim.
    LendWithdraw {
        lender: AccountId,
        program: ProgramId,
        asset: Address,
        shares: Fixed,
        nonce: Hash,
        inputs: Vec<(Hash, Hash)>,
        change: Vec<Output>,
    },
    /// Lock collateral and borrow against it; the borrowed coins are a claim.
    LendOpen {
        borrower: AccountId,
        program: ProgramId,
        collateral_asset: Address,
        collateral: Fixed,
        borrow_asset: Address,
        amount: Fixed,
        nonce: Hash,
        inputs: Vec<(Hash, Hash)>,
        change: Vec<Output>,
    },
    /// Pay down a position; anything over what is owed comes back as a claim.
    LendRepay {
        borrower: AccountId,
        program: ProgramId,
        position: Hash,
        amount: Fixed,
        nonce: Hash,
        inputs: Vec<(Hash, Hash)>,
        change: Vec<Output>,
    },
    LendAddCollateral {
        borrower: AccountId,
        program: ProgramId,
        position: Hash,
        amount: Fixed,
        inputs: Vec<(Hash, Hash)>,
        change: Vec<Output>,
    },
    /// Take collateral out, as a claim, if the position stays within its
    /// maximum.
    LendRemoveCollateral {
        borrower: AccountId,
        program: ProgramId,
        position: Hash,
        amount: Fixed,
        nonce: Hash,
    },
    /// Caps and the smallest loan for an asset. The first one turns guarded
    /// lending on for good (`lend::MAX_PRICE_AGE_EPOCHS` and the rest).
    /// Operator only.
    LendLimits {
        program: ProgramId,
        asset: Address,
        collateral_cap: Fixed,
        borrow_cap: Fixed,
        min_borrow_usd: Fixed,
    },
    /// Switch lending to rules `version` (2: DECISIONS §160), for good.
    /// Operator only.
    LendRules {
        program: ProgramId,
        version: u32,
    },
    /// Rules v2: the operator's clock, posted on its own. Every pool accrues
    /// interest to `at`; no price gets any younger. Operator only.
    LendTime {
        program: ProgramId,
        at: u64,
    },
    /// Rules v2: add `amount` of `asset` to its pool's insurance. No shares,
    /// nothing back.
    LendInsure {
        donor: AccountId,
        program: ProgramId,
        asset: Address,
        amount: Fixed,
        inputs: Vec<(Hash, Hash)>,
        change: Vec<Output>,
    },
    /// Bid `pay` of the borrowed asset for a liquidation lot's collateral at
    /// this epoch's auction price.
    LendBid {
        bidder: AccountId,
        program: ProgramId,
        lot: Hash,
        pay: Fixed,
        nonce: Hash,
        inputs: Vec<(Hash, Hash)>,
        change: Vec<Output>,
    },
    Seal,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Receipt {
    Minted { commitment: Hash, position: u64 },
    Spent { spent: Vec<Hash>, created: Vec<Hash> },
    Deposited { commitment: Hash, position: u64 },
    Withdrawing { amount: Fixed, destination: Hash },
    Registered { account: AccountId },
    Settled { index: u32, txid: Hash },
    Listed { item: Hash },
    Delisted { item: Hash },
    Taken { item: Hash, created: Vec<Hash> },
    /// Liquidity went in; `source` is what its claims derive from.
    Pooled { pool: Hash, shares: Fixed, source: Hash },
    Unpooled { pool: Hash, source: Hash },
    Ordered { order: Hash },
    Claimed { created: Vec<Hash> },
    Launched { token: Address },
    Committed { commit: Hash },
    FeesSet { bridge_bps: u32 },
    CollectionRegistered { asset: Address },
    Redeemed { paid: Fixed, source: Hash },
    CurveCreated { token: Address },
    CurveTermsSet { fee_bps: u32, bond: Fixed },
    CurveQuoteSet { quote: Address, unit: i128, bond: Fixed },
    SaleGated { collection: Address, base: u32 },
    BuyersAllowed { collection: Address, added: u32 },
    SaleOpened { collection: Address },
    TicketsBought { first: Hash, count: u32 },
    SaleClosed { collection: Address, tickets: u32 },
    DrawPosted { collection: Address },
    ItemsClaimed { created: Vec<Hash> },
    OpenMintPosted { collection: Address, count: u32 },
    VoucherCapSet { collection: Address, cap: u32 },
    VoucherPosted { first: Hash, count: u32 },
    BridgedRegistered { asset: Address },
    DepositWaiting { key: Hash, amount: Fixed },
    /// A lending intent applied; `id` is its source (or the receipt asset,
    /// for a listing).
    Lent { id: Hash },
    Refused(&'static str),
    Sealed(Checkpoint),
}

impl Op {
    /// The nullifiers this intent publishes when it is applied.
    pub fn retired(&self) -> Vec<Hash> {
        let pairs: &[(Hash, Hash)] = match self {
            Op::Spend { inputs, .. }
            | Op::Withdraw { inputs, .. }
            | Op::CreatePool { inputs, .. }
            | Op::AddLiquidity { inputs, .. }
            | Op::RemoveLiquidity { inputs, .. }
            | Op::Swap { inputs, .. }
            | Op::Route { inputs, .. }
            | Op::Commit { inputs, .. }
            | Op::CreateCurve { inputs, .. }
            | Op::CurveTrade { inputs, .. }
            | Op::BuyTickets { inputs, .. }
            | Op::ClaimItems { inputs, .. }
            | Op::ClaimVoucherItems { inputs, .. }
            | Op::MintOpen { inputs, .. }
            | Op::LendSupply { inputs, .. }
            | Op::LendWithdraw { inputs, .. }
            | Op::LendOpen { inputs, .. }
            | Op::LendRepay { inputs, .. }
            | Op::LendAddCollateral { inputs, .. }
            | Op::LendInsure { inputs, .. }
            | Op::LendBid { inputs, .. }
            | Op::Redeem { inputs, .. } => inputs,
            Op::TakeOffer {
                item_nullifier,
                payment,
                ..
            } => {
                let mut v = vec![*item_nullifier];
                v.extend(payment.iter().map(|(_, n)| *n));
                return v;
            }
            _ => &[],
        };
        pairs.iter().map(|(_, n)| *n).collect()
    }

    /// The objects this intent creates, each with the ciphertext its owner
    /// finds it by. A wallet discovers nothing a node does not publish, so
    /// every intent that creates one has to be here.
    pub fn created(&self) -> Vec<&Output> {
        match self {
            Op::Spend { outputs, .. }
            | Op::Withdraw { outputs, .. }
            | Op::TakeOffer { outputs, .. }
            | Op::Claim { outputs, .. } => outputs.iter().collect(),
            Op::CreatePool { change, .. }
            | Op::AddLiquidity { change, .. }
            | Op::RemoveLiquidity { change, .. }
            | Op::Swap { change, .. }
            | Op::Route { change, .. }
            | Op::CreateCurve { change, .. }
            | Op::CurveTrade { change, .. }
            | Op::BuyTickets { change, .. }
            | Op::LendSupply { change, .. }
            | Op::LendWithdraw { change, .. }
            | Op::LendOpen { change, .. }
            | Op::LendRepay { change, .. }
            | Op::LendAddCollateral { change, .. }
            | Op::LendInsure { change, .. }
            | Op::LendBid { change, .. }
            | Op::Commit { change, .. } => change.iter().collect(),
            Op::ClaimItems { change, items, .. } | Op::ClaimVoucherItems { change, items, .. } => change.iter().chain(items.iter()).collect(),
            Op::MintOpen { change, item, .. } => change.iter().chain(std::iter::once(item)).collect(),
            Op::ClaimDeposit { output, .. } => vec![output],
            _ => Vec::new(),
        }
    }
}

fn one(r: Result<Receipt, &'static str>) -> Vec<Receipt> {
    vec![r.unwrap_or_else(Receipt::Refused)]
}

/// The two shapes every market intent carries: objects consumed and change.
mod wire {
    use super::*;

    pub fn inputs(e: &mut Encoder, inputs: &[(Hash, Hash)]) {
        e.u32(inputs.len() as u32);
        for (c, n) in inputs {
            e.bytes(c).bytes(n);
        }
    }

    pub fn outputs(e: &mut Encoder, outputs: &[Output]) {
        e.u32(outputs.len() as u32);
        for o in outputs {
            e.bytes(&o.note.asset)
                .fixed(o.note.amount)
                .bytes(&o.note.owner)
                .bytes(&o.note.nonce)
                .bytes(&o.enc.epk)
                .u32(o.enc.ciphertext.len() as u32)
                .bytes(&o.enc.ciphertext);
        }
    }

    pub fn read_inputs(d: &mut Decoder) -> Option<Vec<(Hash, Hash)>> {
        let n = d.u32().ok()? as usize;
        let mut v = Vec::with_capacity(n.min(1024));
        for _ in 0..n {
            v.push((d.hash().ok()?, d.hash().ok()?));
        }
        Some(v)
    }

    /// A Merkle inclusion path: each step's sibling and which side the node is on.
    pub fn proof(e: &mut Encoder, steps: &[zyn_vm::commit::ProofStep]) {
        e.u32(steps.len() as u32);
        for s in steps {
            e.bytes(&s.sibling).bool(s.node_is_right);
        }
    }

    pub fn read_proof(d: &mut Decoder) -> Option<Vec<zyn_vm::commit::ProofStep>> {
        let n = d.u32().ok()? as usize;
        if n > crate::sale::MAX_PROOF {
            return None;
        }
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            let sibling = d.hash().ok()?;
            let node_is_right = match d.u8().ok()? {
                0 => false,
                1 => true,
                _ => return None,
            };
            v.push(zyn_vm::commit::ProofStep { sibling, node_is_right });
        }
        Some(v)
    }

    pub fn read_outputs(d: &mut Decoder) -> Option<Vec<Output>> {
        let n = d.u32().ok()? as usize;
        let mut v = Vec::with_capacity(n.min(1024));
        for _ in 0..n {
            let note = Note {
                asset: d.hash().ok()?,
                amount: d.fixed().ok()?,
                owner: d.account().ok()?,
                nonce: d.hash().ok()?,
            };
            let epk = d.hash().ok()?;
            let len = d.u32().ok()? as usize;
            let ciphertext = d.take_bytes(len).ok()?.to_vec();
            v.push(Output { note, enc: EncryptedNote { epk, ciphertext } });
        }
        Some(v)
    }
}

/// An exit the vault has been told to pay, and has not yet.
///
/// Recorded in state rather than inferred from intents, because the vault
/// operator needs a list they can act on and a verifier needs the same list
/// to check they acted on nothing else. The destination is a commitment, not
/// an address: `H(address ‖ salt)`, so the chain never carries where anyone's
/// money went — the holder reveals the preimage to the vault out of band.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PendingExit {
    pub asset: Address,
    pub amount: Fixed,
    /// `H(zcash address ‖ salt)`.
    pub destination: Hash,
    /// The epoch it was requested in, so a stalled exit is visible.
    pub since_epoch: u64,
    /// The Zcash transaction that paid it, once one has. `None` is a debt.
    pub paid_by: Option<Hash>,
}

/// An object being created, and how its owner will find it.
///
/// One type rather than two parallel vectors: a note and the ciphertext that
/// locates it are the same fact, and a length mismatch between them would be
/// a transfer the recipient never sees (DECISIONS §133.6).
///
/// The VM requires the ciphertext to be present and **cannot check that it
/// opens** — it holds no viewing key. For a deposit the sequencer encrypts and
/// the incentive is aligned; for a transfer a malicious sender can attach
/// garbage and the payment lands where nobody can spend it. That is the
/// sender's own loss to cause, and it is worth saying rather than implying.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Output {
    pub note: Note,
    pub enc: EncryptedNote,
}

/// A listing: terms attached to an object that has not moved.
///
/// The object stays the seller's and stays in `live`. What this adds is a
/// refusal — no ordinary spend may consume it — and an entitlement: whoever
/// pays `price` may (DECISIONS §132.1).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Offer {
    pub seller: AccountId,
    pub price_asset: Address,
    pub price_amount: Fixed,
    /// Last epoch in which this may be taken, inclusive.
    pub expires_at_epoch: u64,
}

/// One live object as the operator sees it.
///
/// This is the tier boundary in one struct: the chain commits
/// `commitment(note)`, and this is the preimage. At tier 3 `amount` becomes a
/// Pedersen commitment and this type stops being readable.
pub type Live = Note;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ObjectVm {
    pub chain_id: u32,
    pub epoch: u64,
    pub seq: u64,
    pub limits: Limits,
    pub parent_root: Hash,
    pub intent_acc: Hash,
    pub epoch_intents: u64,
    /// Authority only: id → spends made. No value.
    pub accounts: BTreeMap<AccountId, u64>,
    /// Append-only, never sorted. Positions are permanent because witnesses
    /// point at them.
    pub commitments: Commitments,
    pub nullifiers: Nullifiers,
    /// commitment → its preimage, for the conservation check. Committed to by
    /// the commitments root, never published as itself.
    pub live: BTreeMap<Hash, Live>,
    /// Issued supply per asset, the quantity conservation is checked against.
    pub issued: BTreeMap<Address, Fixed>,
    /// Where to pay each account. Public: an address is not a secret, and a
    /// payer who cannot read it cannot pay.
    pub viewing: BTreeMap<AccountId, ViewingKey>,
    /// Exits the vault owes and has not paid. Ordered by the sequence they
    /// were requested in, which is the order a settler must honour.
    pub exits: Vec<PendingExit>,
    /// Zcash transactions already credited. One deposit, one object: without
    /// this, replaying the same intent mints the same ZEC twice.
    pub deposits: BTreeSet<Hash>,
    /// Encumbered commitment → its terms. Public, and deliberately so: a
    /// listing nobody can see is not a listing (§132.4). It names the object,
    /// never the owner.
    pub offers: BTreeMap<Hash, Offer>,
    /// Pools, this epoch's orders, what is owed, and sales (`market`).
    pub market: market::Market,
}

impl ObjectVm {
    fn header_leaf(&self) -> Hash {
        let mut e = Encoder::new();
        e.bytes(b"objectvm.header.v1")
            .u32(self.chain_id)
            .u64(self.epoch)
            .u64(self.seq)
            .bytes(&self.parent_root)
            .bytes(&self.intent_acc)
            .u64(self.epoch_intents)
            .u32(self.limits.max_io);
        e.leaf()
    }

    /// The supply section. Public on purpose: how much of an asset exists is
    /// not a secret, and a verifier needs it to check the chain against a
    /// vault's backing.
    fn supply_leaf(&self) -> Hash {
        let mut e = Encoder::new();
        e.bytes(b"objectvm.supply.v1").u32(self.issued.len() as u32);
        for (asset, amount) in &self.issued {
            e.bytes(asset).fixed(*amount);
        }
        // Where to pay an account is consensus too: a replica that disagreed
        // would build deposits nobody could find.
        e.u32(self.viewing.len() as u32);
        for (a, v) in &self.viewing {
            e.bytes(a).bytes(v);
        }
        // What the vault owes is consensus, not bookkeeping: a settler pays
        // from this list and a verifier checks they paid nothing else.
        e.u32(self.exits.len() as u32);
        for x in &self.exits {
            e.bytes(&x.asset)
                .fixed(x.amount)
                .bytes(&x.destination)
                .u64(x.since_epoch)
                .opt_hash(x.paid_by);
        }
        // Credited deposits belong in the root, not merely in memory: a
        // verifier replaying the chain has to reach the same conclusion about
        // which Zcash transactions have already been spent against.
        e.u32(self.deposits.len() as u32);
        for r in &self.deposits {
            e.bytes(r);
        }
        e.leaf()
    }

    /// The listings section. Public by necessity: a market needs strangers to
    /// find the offer. It names the object and the price, never the seller.
    fn offers_leaf(&self) -> Hash {
        let mut e = Encoder::new();
        e.bytes(b"objectvm.offers.v1").u32(self.offers.len() as u32);
        for (item, o) in &self.offers {
            e.bytes(item)
                .bytes(&o.price_asset)
                .fixed(o.price_amount)
                .u64(o.expires_at_epoch);
        }
        e.leaf()
    }

    /// One account, as the state root sees it.
    ///
    /// Commits the registered keys as well as the spend counter. The exit
    /// proof reads this leaf to check that the nullifier key a claimant
    /// presents is the one this account registered, so a leaf that omitted
    /// them would leave that check with nothing to check against.
    /// Every unspent object's commitment, in the order the tree fixes.
    ///
    /// `live` is a `BTreeMap` keyed by commitment, so this is sorted and a
    /// verifier can binary-search it. Sorted rather than insertion-ordered
    /// because this set shrinks as well as grows, and a tree that has to
    /// remove a leaf cannot also promise a permanent position.
    pub fn live_commitments(&self) -> Vec<Hash> {
        self.live.keys().copied().collect()
    }

    /// Does the ledger account for every object it ever created?
    ///
    /// Every commitment ever appended is either still live or has been
    /// retired, and retiring one publishes exactly one nullifier. So:
    ///
    /// ```text
    ///     commitments  ==  live  +  nullifiers
    /// ```
    ///
    /// This is the double-spend check that actually works at this tier, and
    /// it is worth being precise about why the obvious one does not. §130.10
    /// hoped a stranger could "check no nullifier appeared twice" — but a
    /// nullifier is chosen by the spender and the VM cannot verify it derives
    /// from their key, so an operator retiring an object twice would simply
    /// publish two unrelated nullifiers and the check would pass (A120).
    ///
    /// Counting closes that. An object cannot leave the live set without a
    /// nullifier appearing, and a nullifier cannot appear without an object
    /// leaving, so an operator who retires an object quietly — or invents
    /// spends that retired nothing — fails this immediately. The counts are
    /// in the node's status output, so it is checkable by anyone without
    /// holding any key.
    ///
    /// What it does **not** do is tell you *which* object a nullifier
    /// retired. That linkage stays hidden on purpose — see
    /// `a_published_nullifier_does_not_name_its_commitment` — and recovering
    /// it for a verifier without also handing it to every observer is what
    /// A108's circuit is for.
    pub fn reconciled(&self) -> Result<(), &'static str> {
        let created = self.commitments.len();
        let held = self.live.len();
        let retired = self.nullifiers.len();
        if created != held + retired {
            return Err("objects created do not equal those held plus those retired");
        }
        Ok(())
    }

    /// The bytes an account leaf commits to.
    ///
    /// `account_leaf` hashes exactly this and `account_record` serves exactly
    /// this, so the two cannot disagree. They did once: the leaf grew the
    /// registered keys and the record did not, and the conformance suite
    /// caught it (rule S6). One encoding, two callers.
    pub fn account_record_bytes(
        id: &AccountId,
        spends: u64,
        viewing: Option<&ViewingKey>,
    ) -> Vec<u8> {
        let mut e = Encoder::new();
        e.bytes(b"objectvm.account.v2")
            .bytes(id)
            .u64(spends)
            .bytes(viewing.unwrap_or(&[0u8; 32]));
        e.finish().to_vec()
    }

    fn account_leaf(
        id: &AccountId,
        spends: u64,
        viewing: Option<&ViewingKey>,
    ) -> Hash {
        zyn_vm::commit::hash_leaf(&Self::account_record_bytes(id, spends, viewing))
    }

    /// Sum of live amounts for one asset.
    fn held(&self, asset: &Address) -> Option<Fixed> {
        let mut total: i128 = 0;
        for note in self.live.values() {
            if &note.asset == asset {
                total = total.checked_add(note.amount.0)?;
            }
        }
        Some(Fixed(total))
    }

    fn apply_spend(
        &mut self,
        spender: &AccountId,
        program: &ProgramId,
        inputs: &[(Hash, Hash)],
        outputs: &[Output],
    ) -> Vec<Receipt> {
        // An unknown rule is refused rather than ignored: a transaction that
        // otherwise verifies but names a law nobody checked is worse than one
        // that fails. Teaching this VM to accept a program hash is the
        // deliberate act that opens permissionless deployment (§131.4).
        if !program::is_named(program) {
            return vec![Receipt::Refused("unknown program")];
        }
        if *program != program::private_transfer() {
            return vec![Receipt::Refused("this VM implements only private transfer")];
        }
        if inputs.is_empty() || outputs.is_empty() {
            return vec![Receipt::Refused("a spend consumes and creates")];
        }
        if inputs.len() as u32 > self.limits.max_io || outputs.len() as u32 > self.limits.max_io {
            return vec![Receipt::Refused("too many objects in one spend")];
        }

        // Every input must exist, be unspent, and be the spender's. Checked
        // before anything mutates, so a refusal leaves the tree byte-identical
        // — which is what conformance S3 requires of a rejected batch.
        let mut consumed: Vec<Note> = Vec::with_capacity(inputs.len());
        let mut seen: Vec<Hash> = Vec::with_capacity(inputs.len());
        for (c, n) in inputs {
            if seen.contains(c) {
                return vec![Receipt::Refused("the same object twice in one spend")];
            }
            seen.push(*c);
            let note = match self.live.get(c) {
                Some(note) => *note,
                None => return vec![Receipt::Refused("no such live object")],
            };
            if &note.owner != spender {
                return vec![Receipt::Refused("not the spender's object")];
            }
            if self.nullifiers.contains(n) {
                return vec![Receipt::Refused("already spent")];
            }
            // The encumbrance in one line: it is still yours, and you still
            // cannot spend it while it is listed (§132.1).
            if self.offers.contains_key(c) {
                return vec![Receipt::Refused("that object is listed")];
            }
            consumed.push(note);
        }

        // Outputs must be committable at all.
        let mut created: Vec<(Hash, Note)> = Vec::with_capacity(outputs.len());
        for out in outputs {
            // An output nobody can find is value destroyed in place.
            if out.enc.ciphertext.is_empty() {
                return vec![Receipt::Refused("an output needs an encrypted note")];
            }
            match commitment(&out.note) {
                Ok(c) => {
                    if self.live.contains_key(&c) {
                        return vec![Receipt::Refused("output already exists")];
                    }
                    created.push((c, out.note))
                }
                Err(ObjectError::ZeroNonce) => {
                    return vec![Receipt::Refused("an output needs a nonce")]
                }
                Err(ObjectError::NotPositive) => {
                    return vec![Receipt::Refused("an output must be worth something")]
                }
            }
        }

        // Value in equals value out, per asset. This is `conserved()` in the
        // small: the batch-level check would catch a violation too, but a
        // whole batch would be discarded for one bad intent.
        let mut delta: BTreeMap<Address, i128> = BTreeMap::new();
        for note in &consumed {
            let e = delta.entry(note.asset).or_insert(0);
            *e = match e.checked_add(note.amount.0) {
                Some(v) => v,
                None => return vec![Receipt::Refused("input overflow")],
            };
        }
        for (_, note) in &created {
            let e = delta.entry(note.asset).or_insert(0);
            *e = match e.checked_sub(note.amount.0) {
                Some(v) => v,
                None => return vec![Receipt::Refused("output overflow")],
            };
        }
        if delta.values().any(|d| *d != 0) {
            return vec![Receipt::Refused("value not conserved")];
        }

        // Only now does anything move.
        for (c, n) in inputs {
            self.live.remove(c);
            self.nullifiers.insert(*n);
        }
        let mut made = Vec::with_capacity(created.len());
        for (c, note) in created {
            self.commitments.append(c);
            self.live.insert(c, note);
            made.push(c);
        }
        *self.accounts.entry(*spender).or_insert(0) += 1;

        vec![Receipt::Spent {
            spent: inputs.iter().map(|(_, n)| *n).collect(),
            created: made,
        }]
    }
}

impl ObjectVm {
    /// Pay the price and take the object, in one step.
    ///
    /// Atomicity is the whole design (§132.2): the payment and the transfer
    /// are the same transaction, so a buyer cannot pay without receiving, and
    /// a cancel that the sequencer ordered first simply makes this fail with
    /// the payment still the buyer's. No lock, no staging, no refund path.
    ///
    /// This is the one rule permitted to consume an object it does not own.
    /// The entitlement comes from the encumbrance, and the price is what
    /// converts it into a right.
    fn apply_take(
        &mut self,
        buyer: &AccountId,
        program: &ProgramId,
        item: &Hash,
        item_nullifier: &Hash,
        payment: &[(Hash, Hash)],
        outputs: &[Output],
    ) -> Vec<Receipt> {
        if *program != program::take_offer() {
            return vec![Receipt::Refused("wrong program for a claim")];
        }
        let offer = match self.offers.get(item) {
            Some(o) => *o,
            None => return vec![Receipt::Refused("no such listing")],
        };
        if self.epoch > offer.expires_at_epoch {
            return vec![Receipt::Refused("the listing expired")];
        }
        let item_note = match self.live.get(item) {
            Some(n) => *n,
            None => return vec![Receipt::Refused("the listed object is gone")],
        };
        if self.nullifiers.contains(item_nullifier) {
            return vec![Receipt::Refused("already spent")];
        }
        if payment.len() as u32 > self.limits.max_io || outputs.len() as u32 > self.limits.max_io {
            return vec![Receipt::Refused("too many objects in one claim")];
        }

        // The buyer funds it with objects that are theirs, live, unlisted.
        let mut consumed: Vec<Note> = alloc_vec![item_note];
        let mut seen: Vec<Hash> = alloc_vec![*item];
        for (c, n) in payment {
            if seen.contains(c) {
                return vec![Receipt::Refused("the same object twice in one claim")];
            }
            seen.push(*c);
            let note = match self.live.get(c) {
                Some(note) => *note,
                None => return vec![Receipt::Refused("no such live object")],
            };
            if &note.owner != buyer {
                return vec![Receipt::Refused("not the buyer's object")];
            }
            if self.offers.contains_key(c) {
                return vec![Receipt::Refused("that object is listed")];
            }
            if self.nullifiers.contains(n) {
                return vec![Receipt::Refused("already spent")];
            }
            consumed.push(note);
        }

        let mut created: Vec<(Hash, Note)> = Vec::with_capacity(outputs.len());
        for out in outputs {
            if out.enc.ciphertext.is_empty() {
                return vec![Receipt::Refused("an output needs an encrypted note")];
            }
            match commitment(&out.note) {
                Ok(c) => {
                    if self.live.contains_key(&c) {
                        return vec![Receipt::Refused("output already exists")];
                    }
                    created.push((c, out.note))
                }
                Err(_) => return vec![Receipt::Refused("an output needs a nonce and a value")],
            }
        }

        // The seller is paid at least what was asked, and the buyer receives
        // the object itself rather than something merely similar.
        let mut paid: i128 = 0;
        for (_, n) in &created {
            if n.owner == offer.seller && n.asset == offer.price_asset {
                paid = match paid.checked_add(n.amount.0) {
                    Some(v) => v,
                    // Unreachable while every amount is positive and bounded
                    // by conservation — but `sum()` here used to panic on
                    // attacker-chosen outputs, and a halt is not a refusal.
                    None => return vec![Receipt::Refused("payment overflow")],
                };
            }
        }
        // A collection with terms charges each side `fee_bps` of the price:
        // the seller out of the proceeds, the buyer on top (DECISIONS §69.9).
        let terms = self.market.terms_for(&item_note.asset).map(|(_, t)| t);
        let collection = self.market.items.get(&item_note.asset).map(|i| i.collection);
        // A `List`ed item pays its seller by claim; the buyer's outputs are
        // the item and their own change, and never name the seller.
        let listing = self.market.listings.get(item).copied();
        if let Some(l) = &listing {
            if *item_nullifier != l.nullifier {
                return vec![Receipt::Refused("that is not the listed item's nullifier")];
            }
        }
        let each = match &terms {
            Some(t) => {
                if offer.price_asset != t.quote {
                    return vec![Receipt::Refused("this collection trades in its quote asset")];
                }
                match market::fee_of(offer.price_amount, t.fee_bps) {
                    Some(f) => f,
                    None => return vec![Receipt::Refused("fee overflow")],
                }
            }
            None => Fixed::ZERO,
        };
        if listing.is_none() && paid < offer.price_amount.0 - each.0 {
            return vec![Receipt::Refused("the seller was not paid the asking price")];
        }
        if !created.iter().any(|(_, n)| {
            n.owner == *buyer && n.asset == item_note.asset && n.amount == item_note.amount
        }) {
            return vec![Receipt::Refused("the buyer does not receive the object")];
        }

        // And nothing was created from nothing.
        let mut delta: BTreeMap<Address, i128> = BTreeMap::new();
        for note in &consumed {
            let e = delta.entry(note.asset).or_insert(0);
            *e = match e.checked_add(note.amount.0) {
                Some(v) => v,
                None => return vec![Receipt::Refused("input overflow")],
            };
        }
        for (_, note) in &created {
            let e = delta.entry(note.asset).or_insert(0);
            *e = match e.checked_sub(note.amount.0) {
                Some(v) => v,
                None => return vec![Receipt::Refused("output overflow")],
            };
        }
        // Both sides' fees leave the traders for the pots, and a listed
        // item's proceeds leave them for the seller's claim.
        let into_market = each.0 * 2 + if listing.is_some() { offer.price_amount.0 - each.0 } else { 0 };
        if into_market > 0 {
            let q = delta.entry(offer.price_asset).or_insert(0);
            *q = match q.checked_sub(into_market) {
                Some(v) => v,
                None => return vec![Receipt::Refused("fee overflow")],
            };
        }
        if delta.values().any(|d| *d != 0) {
            return vec![Receipt::Refused("value not conserved")];
        }

        // Only now.
        self.offers.remove(item);
        self.live.remove(item);
        self.nullifiers.insert(*item_nullifier);
        for (c, n) in payment {
            self.live.remove(c);
            self.nullifiers.insert(*n);
        }
        let mut made = Vec::with_capacity(created.len());
        for (c, note) in created {
            self.commitments.append(c);
            self.live.insert(c, note);
            made.push(c);
        }
        *self.accounts.entry(*buyer).or_insert(0) += 1;
        self.accounts.entry(offer.seller).or_insert(0);
        if let (Some(t), Some(coll), true) = (terms, collection, each.0 > 0) {
            let (to_pol, to_floor, to_creator) = market::resale_split(Fixed(each.0 * 2));
            let _ = self.market.to_pol(t.quote, to_pol);
            self.market
                .collections
                .insert(coll, market::CollectionTerms { floor: Fixed(t.floor.0 + to_floor.0), ..t });
            self.market.pay(&market::sale_fee_source(item), 0, t.creator, t.quote, to_creator);
        }
        if let Some(l) = listing {
            self.market.listings.remove(item);
            self.market.pay(&l.source, 0, offer.seller, offer.price_asset, Fixed(offer.price_amount.0 - each.0));
        }

        vec![Receipt::Taken {
            item: *item,
            created: made,
        }]
    }
}

impl ObjectVm {
    /// Burn objects and record what the vault owes.
    ///
    /// Supply falls by exactly what leaves. A deposit that minted more than
    /// the vault received and a withdrawal that burned less than it paid out
    /// are the same bug from opposite ends, and `conserved()` catches neither
    /// on its own — it checks that live objects sum to *issued*, so issued has
    /// to move in step or the books balance against the wrong number.
    #[allow(clippy::too_many_arguments)]
    fn apply_withdraw(
        &mut self,
        holder: &AccountId,
        program: &ProgramId,
        inputs: &[(Hash, Hash)],
        asset: &Address,
        amount: &Fixed,
        destination: &Hash,
        outputs: &[Output],
    ) -> Vec<Receipt> {
        if *program != program::withdraw() {
            return vec![Receipt::Refused("wrong program for a withdrawal")];
        }
        // Not merely non-zero. A *negative* withdrawal subtracts a negative
        // from `issued`, which raises the supply the vault believes it backs,
        // and records a debt of less than nothing. Do it once and the chain
        // will let you withdraw the difference for real (§136.6).
        if amount.0 <= 0 {
            return vec![Receipt::Refused("a withdrawal must be of something")];
        }
        if *destination == [0u8; 32] {
            return vec![Receipt::Refused("a withdrawal needs a destination")];
        }
        // Pool shares and launched tokens were made here and nothing in the
        // vault backs them. Letting one "leave" would record a debt in an
        // asset the vault does not hold.
        if self.market.is_derived(asset) {
            return vec![Receipt::Refused("only deposited assets leave for Zcash")];
        }
        if inputs.is_empty() {
            return vec![Receipt::Refused("a withdrawal consumes something")];
        }
        if inputs.len() as u32 > self.limits.max_io || outputs.len() as u32 > self.limits.max_io {
            return vec![Receipt::Refused("too many objects in one withdrawal")];
        }

        let mut consumed: Vec<Note> = Vec::with_capacity(inputs.len());
        let mut seen: Vec<Hash> = Vec::with_capacity(inputs.len());
        for (c, n) in inputs {
            if seen.contains(c) {
                return vec![Receipt::Refused("the same object twice in one withdrawal")];
            }
            seen.push(*c);
            let note = match self.live.get(c) {
                Some(note) => *note,
                None => return vec![Receipt::Refused("no such live object")],
            };
            if &note.owner != holder {
                return vec![Receipt::Refused("not the holder's object")];
            }
            if self.offers.contains_key(c) {
                return vec![Receipt::Refused("that object is listed")];
            }
            if self.nullifiers.contains(n) {
                return vec![Receipt::Refused("already spent")];
            }
            consumed.push(note);
        }

        let mut created: Vec<(Hash, Note)> = Vec::with_capacity(outputs.len());
        for out in outputs {
            if out.enc.ciphertext.is_empty() {
                return vec![Receipt::Refused("an output needs an encrypted note")];
            }
            match commitment(&out.note) {
                Ok(c) => {
                    if self.live.contains_key(&c) {
                        return vec![Receipt::Refused("output already exists")];
                    }
                    created.push((c, out.note))
                }
                Err(_) => return vec![Receipt::Refused("an output needs a nonce and a value")],
            }
        }

        // In equals out, plus what leaves. The leaving amount is the only
        // place value may vanish, and only for the asset named.
        let mut delta: BTreeMap<Address, i128> = BTreeMap::new();
        for note in &consumed {
            let e = delta.entry(note.asset).or_insert(0);
            *e = match e.checked_add(note.amount.0) {
                Some(v) => v,
                None => return vec![Receipt::Refused("input overflow")],
            };
        }
        for (_, note) in &created {
            let e = delta.entry(note.asset).or_insert(0);
            *e = match e.checked_sub(note.amount.0) {
                Some(v) => v,
                None => return vec![Receipt::Refused("output overflow")],
            };
        }
        // The bridge fee rides on top of what leaves, and stays behind in
        // the ZYN pot — still issued, still backed, just no longer anyone's.
        let Some(fee) = market::fee_of(*amount, self.market.bridge_bps()) else {
            return vec![Receipt::Refused("fee overflow")];
        };
        let leaving = delta.entry(*asset).or_insert(0);
        *leaving = match leaving.checked_sub(amount.0).and_then(|v| v.checked_sub(fee.0)) {
            Some(v) => v,
            None => return vec![Receipt::Refused("withdrawal overflow")],
        };
        if delta.values().any(|d| *d != 0) {
            return vec![Receipt::Refused("value not conserved")];
        }

        // Supply must fall with it, or the chain claims backing it gave away.
        let supply = self.issued.entry(*asset).or_insert(Fixed::ZERO);
        *supply = match supply.0.checked_sub(amount.0) {
            Some(v) if v >= 0 => Fixed(v),
            _ => return vec![Receipt::Refused("withdrawal exceeds issued supply")],
        };

        for (c, n) in inputs {
            self.live.remove(c);
            self.nullifiers.insert(*n);
        }
        for (c, note) in created {
            self.commitments.append(c);
            self.live.insert(c, note);
        }
        let _ = self.market.to_pol(*asset, fee);
        self.exits.push(PendingExit {
            asset: *asset,
            amount: *amount,
            destination: *destination,
            since_epoch: self.epoch,
            paid_by: None,
        });
        *self.accounts.entry(*holder).or_insert(0) += 1;

        vec![Receipt::Withdrawing {
            amount: *amount,
            destination: *destination,
        }]
    }
}

impl ObjectVm {
    /// Credit one deposit: `amount` to its owner as a note, `fee` to the ZYN
    /// pot, and issued supply up by `gross` — what the vault actually holds.
    #[allow(clippy::too_many_arguments)]
    fn credit_deposit(
        &mut self,
        asset: Address,
        gross: Fixed,
        amount: Fixed,
        fee: Fixed,
        owner: &AccountId,
        nonce: &Hash,
        enc: &EncryptedNote,
        external_ref: &Hash,
    ) -> Vec<Receipt> {
        let asset = &asset;
        let amount = &amount;
        if *external_ref == [0u8; 32] {
            return vec![Receipt::Refused("a deposit names the transaction behind it")];
        }
        // A deposit nobody can find is a deposit nobody can spend. The
        // VM cannot check that the ciphertext opens to this note — it
        // holds no viewing key — but it can refuse an absent one.
        if enc.ciphertext.is_empty() {
            return vec![Receipt::Refused("a deposit needs an encrypted note")];
        }
        if self.deposits.contains(external_ref) {
            return vec![Receipt::Refused("that deposit was already credited")];
        }
        let note = Note {
            asset: *asset,
            amount: *amount,
            owner: *owner,
            nonce: *nonce,
        };
        let c = match commitment(&note) {
            Ok(c) => c,
            Err(_) => return vec![Receipt::Refused("a deposit needs a nonce and a value")],
        };
        if self.live.contains_key(&c) {
            return vec![Receipt::Refused("that object already exists")];
        }
        if self
            .market
            .pol
            .get(asset)
            .map_or(0, |p| p.0)
            .checked_add(fee.0)
            .is_none()
        {
            return vec![Receipt::Refused("pot overflow")];
        }
        let supply = self.issued.entry(*asset).or_insert(Fixed::ZERO);
        *supply = match supply.0.checked_add(gross.0) {
            Some(v) => Fixed(v),
            None => return vec![Receipt::Refused("supply overflow")],
        };
        let position = self.commitments.append(c) as u64;
        self.live.insert(c, note);
        self.deposits.insert(*external_ref);
        self.accounts.entry(*owner).or_insert(0);
        let _ = self.market.to_pol(*asset, fee);
        vec![Receipt::Deposited {
            commitment: c,
            position,
        }]
    }
}

impl ObjectVm {
    /// The checks and the encumbrance every listing shares.
    fn list(
        &mut self,
        seller: &AccountId,
        item: &Hash,
        price_asset: &Address,
        price_amount: &Fixed,
        expires_at_epoch: &u64,
    ) -> Result<(), &'static str> {

                let note = match self.live.get(item) {
                    Some(n) => *n,
                    None => return Err("no such live object"),
                };
                if &note.owner != seller {
                    return Err("not the seller's object");
                }
                if self.offers.contains_key(item) {
                    return Err("already listed");
                }
                if let Some((_, t)) = self.market.terms_for(&note.asset) {
                    if *price_asset != t.quote {
                        return Err("this collection trades in its quote asset");
                    }
                }
                // A negative asking price is satisfied by any payment at
                // all, including none, because the claim path only checks
                // the seller was paid *at least* this much.
                if price_amount.0 <= 0 {
                    return Err("a listing needs a price above nothing");
                }
                if *expires_at_epoch < self.epoch {
                    return Err("already expired");
                }
                self.offers.insert(
                    *item,
                    Offer {
                        seller: *seller,
                        price_asset: *price_asset,
                        price_amount: *price_amount,
                        expires_at_epoch: *expires_at_epoch,
                    },
                );
        Ok(())
    }
}

impl MicrochainVm for ObjectVm {
    type Intent = Op;
    type Receipt = Receipt;
    type Params = Limits;

    const VM_NAME: &'static str = "objectvm";
    const VM_VERSION: u16 = 1;

    fn genesis(chain_id: u32, limits: Limits) -> Self {
        ObjectVm {
            chain_id,
            epoch: 0,
            seq: 0,
            limits,
            parent_root: [0u8; 32],
            intent_acc: [0u8; 32],
            epoch_intents: 0,
            accounts: BTreeMap::new(),
            commitments: Commitments::new(),
            nullifiers: Nullifiers::new(),
            live: BTreeMap::new(),
            issued: BTreeMap::new(),
            offers: BTreeMap::new(),
            deposits: BTreeSet::new(),
            exits: Vec::new(),
            viewing: BTreeMap::new(),
            market: market::Market::default(),
        }
    }

    fn chain_id(&self) -> u32 {
        self.chain_id
    }

    fn seq(&self) -> u64 {
        self.seq
    }

    fn epoch(&self) -> u64 {
        self.epoch
    }

    fn apply(&mut self, seq: u64, intent: &Op) -> Vec<Receipt> {
        if seq != self.seq + 1 {
            return vec![Receipt::Refused("out of order")];
        }
        self.seq = seq;
        self.intent_acc = zyn_vm::fold_intent(self.intent_acc, seq, &Self::encode_intent(intent));
        self.epoch_intents += 1;

        match intent {
            Op::Mint {
                asset,
                amount,
                owner,
                nonce,
            } => {
                let note = Note {
                    asset: *asset,
                    amount: *amount,
                    owner: *owner,
                    nonce: *nonce,
                };
                let c = match commitment(&note) {
                    Ok(c) => c,
                    Err(ObjectError::ZeroNonce) => {
                        return vec![Receipt::Refused("a mint needs a nonce")]
                    }
                    Err(ObjectError::NotPositive) => {
                        return vec![Receipt::Refused("a mint of nothing")]
                    }
                };
                if self.live.contains_key(&c) {
                    return vec![Receipt::Refused("that object already exists")];
                }
                let supply = self.issued.entry(*asset).or_insert(Fixed::ZERO);
                *supply = match supply.0.checked_add(amount.0) {
                    Some(v) => Fixed(v),
                    None => return vec![Receipt::Refused("supply overflow")],
                };
                let position = self.commitments.append(c) as u64;
                self.live.insert(c, note);
                self.accounts.entry(*owner).or_insert(0);
                vec![Receipt::Minted {
                    commitment: c,
                    position,
                }]
            }
            Op::Spend {
                spender,
                program,
                inputs,
                outputs,
            } => self.apply_spend(spender, program, inputs, outputs),
            Op::SettleExit {
                program,
                index,
                txid,
            } => {
                if *program != program::settle_exit() {
                    return vec![Receipt::Refused("wrong program for a settlement")];
                }
                if *txid == [0u8; 32] {
                    return vec![Receipt::Refused("a settlement names its transaction")];
                }
                let Some(exit) = self.exits.get_mut(*index as usize) else {
                    return vec![Receipt::Refused("no such exit")];
                };
                // Marking a debt paid twice would let an operator answer two
                // obligations with one transaction and appear square.
                if exit.paid_by.is_some() {
                    return vec![Receipt::Refused("that exit was already settled")];
                }
                exit.paid_by = Some(*txid);
                vec![Receipt::Settled {
                    index: *index,
                    txid: *txid,
                }]
            }
            Op::Register {
                account,
                program,
                viewing,
            } => {
                if *program != program::register_viewing_key() {
                    return vec![Receipt::Refused("wrong program for a registration")];
                }
                if *viewing == [0u8; 32] {
                    return vec![Receipt::Refused("not a viewing key")];
                }
                self.viewing.insert(*account, *viewing);
                self.accounts.entry(*account).or_insert(0);
                vec![Receipt::Registered { account: *account }]
            }
            Op::Withdraw {
                holder,
                program,
                inputs,
                asset,
                amount,
                destination,
                outputs,
            } => self.apply_withdraw(holder, program, inputs, asset, amount, destination, outputs),
            Op::Deposit {
                program,
                asset,
                amount,
                owner,
                nonce,
                enc,
                external_ref,
            } => {
                if *program != program::deposit() {
                    return vec![Receipt::Refused("wrong program for a deposit")];
                }
                // Once the bridge charges, a credit that skips the fee is a
                // free channel around it.
                if self.market.bridge_bps() > 0 {
                    return vec![Receipt::Refused("the bridge charges a fee; credit it with DepositWithFee")];
                }
                self.credit_deposit(*asset, *amount, *amount, Fixed::ZERO, owner, nonce, enc, external_ref)
            }
            Op::PlaceOffer {
                seller,
                program,
                item,
                price_asset,
                price_amount,
                expires_at_epoch,
            } => {
                if *program != program::place_offer() {
                    return vec![Receipt::Refused("wrong program for a listing")];
                }
                // A collection's items sell through `List`, which pays the
                // seller by claim and publishes the nullifier the sale needs.
                if let Some(n) = self.live.get(item) {
                    if self.market.items.contains_key(&n.asset) {
                        return vec![Receipt::Refused("list a collection's items with List")];
                    }
                }
                if let Err(why) = self.list(seller, item, price_asset, price_amount, expires_at_epoch) {
                    return vec![Receipt::Refused(why)];
                }
                vec![Receipt::Listed { item: *item }]
            }
            Op::CancelOffer {
                seller,
                program,
                item,
            } => {
                if *program != program::cancel_offer() {
                    return vec![Receipt::Refused("wrong program for a cancel")];
                }
                match self.offers.get(item) {
                    Some(o) if &o.seller == seller => {
                        self.offers.remove(item);
                        self.market.listings.remove(item);
                        vec![Receipt::Delisted { item: *item }]
                    }
                    Some(_) => vec![Receipt::Refused("not the seller's listing")],
                    None => vec![Receipt::Refused("no such listing")],
                }
            }
            Op::TakeOffer {
                buyer,
                program,
                item,
                item_nullifier,
                payment,
                outputs,
            } => self.apply_take(buyer, program, item, item_nullifier, payment, outputs),
            Op::CreatePool {
                provider,
                program,
                asset_a,
                asset_b,
                amount_a,
                amount_b,
                fee_bps,
                inputs,
                change,
                nonce,
            } => one(if *program != program::add_liquidity() {
                Err("wrong program for a pool")
            } else {
                self.apply_create_pool(
                    provider, asset_a, asset_b, *amount_a, *amount_b, *fee_bps, inputs, change, nonce,
                )
            }),
            Op::AddLiquidity {
                provider,
                program,
                pool,
                amount_a,
                amount_b,
                min_shares,
                inputs,
                change,
                nonce,
            } => one(if *program != program::add_liquidity() {
                Err("wrong program for liquidity")
            } else {
                self.apply_add_liquidity(
                    provider, pool, *amount_a, *amount_b, *min_shares, inputs, change, nonce,
                )
            }),
            Op::RemoveLiquidity {
                provider,
                program,
                pool,
                shares,
                min_a,
                min_b,
                inputs,
                change,
                nonce,
            } => one(if *program != program::remove_liquidity() {
                Err("wrong program for liquidity")
            } else {
                self.apply_remove_liquidity(provider, pool, *shares, *min_a, *min_b, inputs, change, nonce)
            }),
            Op::Swap {
                trader,
                program,
                pool,
                sell_a,
                amount_in,
                min_out,
                inputs,
                change,
                nonce,
            } => one(if *program != program::amm_swap() {
                Err("wrong program for a swap")
            } else {
                self.apply_swap(trader, pool, *sell_a, *amount_in, *min_out, inputs, change, nonce)
            }),
            Op::Route { trader, program, hops, amount_in, min_out, inputs, change, nonce } => one(if *program != program::route() {
                Err("wrong program for a route")
            } else {
                self.apply_route(trader, hops, *amount_in, *min_out, inputs, change, nonce)
            }),
            Op::Claim {
                owner,
                program,
                claims,
                outputs,
            } => one(if *program != program::claim() {
                Err("wrong program for a claim")
            } else {
                self.apply_claim(owner, claims, outputs)
            }),
            Op::CreateLaunch {
                creator,
                program,
                nonce,
                quote,
                total_supply,
                sale_supply,
                pool_supply,
                soft_cap,
                hard_cap,
                pool_bps,
                fee_bps,
                closes_at_epoch,
                meta,
            } => one(if *program != program::launch_create() {
                Err("wrong program for a launch")
            } else {
                self.apply_create_launch(
                    creator,
                    nonce,
                    quote,
                    *total_supply,
                    *sale_supply,
                    *pool_supply,
                    *soft_cap,
                    *hard_cap,
                    *pool_bps,
                    *fee_bps,
                    *closes_at_epoch,
                    meta,
                )
            }),
            Op::Commit {
                buyer,
                program,
                launch,
                amount,
                inputs,
                change,
                nonce,
            } => one(if *program != program::launch_commit() {
                Err("wrong program for a commitment")
            } else {
                self.apply_commit(buyer, launch, *amount, inputs, change, nonce)
            }),
            Op::SetFees { program, bridge_bps } => {
                if *program != program::configure_fees() {
                    return vec![Receipt::Refused("wrong program for fees")];
                }
                if *bridge_bps > market::MAX_BRIDGE_BPS {
                    return vec![Receipt::Refused("the bridge fee is above the ceiling")];
                }
                self.market.schedule = Some(market::FeeSchedule { bridge_bps: *bridge_bps });
                vec![Receipt::FeesSet { bridge_bps: *bridge_bps }]
            }
            Op::RegisterCollection { program, asset, creator, quote, fee_bps, cap } => {
                if *program != program::register_collection() {
                    return vec![Receipt::Refused("wrong program for a collection")];
                }
                if *fee_bps > market::MAX_COLLECTION_FEE_BPS {
                    return vec![Receipt::Refused("the resale fee is above the ceiling")];
                }
                if asset == quote {
                    return vec![Receipt::Refused("a collection is not priced in itself")];
                }
                if self.market.collections.contains_key(asset) {
                    return vec![Receipt::Refused("that collection already has its terms")];
                }
                if *cap == 0 {
                    return vec![Receipt::Refused("a collection has at least one item")];
                }
                self.market.collections.insert(
                    *asset,
                    market::CollectionTerms {
                        creator: *creator,
                        quote: *quote,
                        fee_bps: *fee_bps,
                        floor: Fixed::ZERO,
                        cap: *cap,
                        minted: 0,
                        outstanding: 0,
                    },
                );
                vec![Receipt::CollectionRegistered { asset: *asset }]
            }
            Op::DepositWithFee { program, asset, gross, amount, owner, nonce, enc, external_ref } => {
                if *program != program::deposit() {
                    return vec![Receipt::Refused("wrong program for a deposit")];
                }
                let Some(fee) = market::fee_of(*gross, self.market.bridge_bps()) else {
                    return vec![Receipt::Refused("fee overflow")];
                };
                if gross.0 <= 0 || amount.0 != gross.0 - fee.0 || amount.0 <= 0 {
                    return vec![Receipt::Refused("the credit is not the deposit less the bridge fee")];
                }
                self.credit_deposit(*asset, *gross, *amount, fee, owner, nonce, enc, external_ref)
            }
            Op::List {
                seller,
                program,
                item,
                item_nullifier,
                price_asset,
                price_amount,
                expires_at_epoch,
                nonce,
            } => {
                if *program != program::place_offer() {
                    return vec![Receipt::Refused("wrong program for a listing")];
                }
                if self.nullifiers.contains(item_nullifier) || *item_nullifier == [0u8; 32] {
                    return vec![Receipt::Refused("that nullifier is spent")];
                }
                let source = market::listing_source(seller, nonce);
                if self.market.listings.values().any(|l| l.source == source)
                    || self.market.claims.contains_key(&market::claim_id(&source, 0))
                {
                    return vec![Receipt::Refused("that nonce is in use; list with a fresh one")];
                }
                if let Err(why) = self.list(seller, item, price_asset, price_amount, expires_at_epoch) {
                    return vec![Receipt::Refused(why)];
                }
                self.market.listings.insert(
                    *item,
                    market::Listing { source: market::listing_source(seller, nonce), nullifier: *item_nullifier },
                );
                vec![Receipt::Listed { item: *item }]
            }
            Op::MintItem { program, collection, serial, owner, nonce, enc } => one(if *program != program::mint_item() {
                Err("wrong program for a mint")
            } else {
                self.apply_mint_item(collection, *serial, owner, nonce, enc)
            }),
            Op::Redeem { holder, program, asset, inputs } => one(if *program != program::redeem_item() {
                Err("wrong program for a redemption")
            } else {
                self.apply_redeem(holder, asset, inputs)
            }),
            Op::CreateCurve { creator, program, nonce, quote, meta, inputs, change } => {
                one(if *program != program::curve_create() {
                    Err("wrong program for a curve")
                } else {
                    self.apply_create_curve(creator, nonce, quote, meta, inputs, change)
                })
            }
            Op::CurveTrade { trader, program, token, buy, amount_in, min_out, inputs, change, nonce } => {
                one(if *program != program::curve_trade() {
                    Err("wrong program for a curve trade")
                } else {
                    self.apply_curve_trade(trader, token, *buy, *amount_in, *min_out, inputs, change, nonce)
                })
            }
            Op::SetCurveTerms { program, fee_bps, bond } => {
                if *program != program::configure_curves() {
                    return vec![Receipt::Refused("wrong program for curve terms")];
                }
                if *fee_bps > curve::MAX_CURVE_FEE_BPS {
                    return vec![Receipt::Refused("the curve fee is above the ceiling")];
                }
                if bond.0 < 0 || *bond > curve::MAX_BOND {
                    return vec![Receipt::Refused("the bond is outside what a creator can be asked for")];
                }
                self.market.curve_terms = Some(curve::CurveTerms { fee_bps: *fee_bps, bond: *bond });
                vec![Receipt::CurveTermsSet { fee_bps: *fee_bps, bond: *bond }]
            }
            Op::GateSale { program, collection, base } => one(if *program != program::sale_open() {
                Err("wrong program for a sale's gate")
            } else {
                self.apply_gate_sale(collection, *base)
            }),
            Op::AllowBuyers { program, collection, keys } => one(if *program != program::sale_open() {
                Err("wrong program for a sale's list")
            } else {
                self.apply_allow_buyers(collection, keys)
            }),
            Op::SetCurveQuote { program, quote, graduation_mc, bond } => {
                if *program != program::configure_curves() {
                    return vec![Receipt::Refused("wrong program for curve terms")];
                }
                if self.market.is_derived(quote) {
                    return vec![Receipt::Refused("coins launch in an asset people deposit, not a share or a coin")];
                }
                if graduation_mc.0 == 0 {
                    // Stops new coins in it; coins already made keep their size.
                    return match self.market.curve_quotes.remove(quote) {
                        Some(_) => vec![Receipt::CurveQuoteSet { quote: *quote, unit: 0, bond: Fixed::ZERO }],
                        None => vec![Receipt::Refused("coins are not launched in that asset")],
                    };
                }
                let q = curve::QuoteTerms { unit: curve::QuoteTerms::unit_for(*graduation_mc), bond: *bond };
                if graduation_mc.0 < 0 || !q.valid() {
                    return vec![Receipt::Refused("those terms are outside what a curve can be sized to")];
                }
                self.market.curve_quotes.insert(*quote, q);
                vec![Receipt::CurveQuoteSet { quote: *quote, unit: q.unit, bond: q.bond }]
            }
            Op::SetSale { program, collection, ticket_price, claim_price, open_price, treasury_bps } => one(if *program != program::sale_open() {
                Err("wrong program for a sale")
            } else {
                self.apply_set_sale(collection, *ticket_price, *claim_price, *open_price, *treasury_bps)
            }),
            Op::BuyTickets { buyer, program, collection, count, nonce, inputs, change } => one(if *program != program::sale_tickets() {
                Err("wrong program for tickets")
            } else {
                self.apply_buy_tickets(buyer, collection, *count, nonce, inputs, change)
            }),
            Op::CloseSale { program, collection, zcash_height, zcash_hash } => one(if *program != program::sale_close() {
                Err("wrong program for a close")
            } else {
                self.apply_close_sale(collection, *zcash_height, zcash_hash)
            }),
            Op::PostDraw { program, collection, root } => one(if *program != program::sale_draw() {
                Err("wrong program for a draw")
            } else {
                self.apply_post_draw(collection, root)
            }),
            Op::ClaimItems { buyer, program, collection, nonce, k, serials, proof, items, inputs, change } => {
                one(if *program != program::sale_claim() {
                    Err("wrong program for a claim")
                } else {
                    self.apply_claim_items(buyer, collection, nonce, *k, serials, proof, items, inputs, change)
                })
            }
            Op::PostOpenMint { program, collection, root, count } => one(if *program != program::sale_open_mint() {
                Err("wrong program for an open mint")
            } else {
                self.apply_post_open_mint(collection, root, *count)
            }),
            Op::MintOpen { buyer, program, collection, serial, proof, item, nonce, inputs, change } => {
                one(if *program != program::sale_mint() {
                    Err("wrong program for a mint")
                } else {
                    self.apply_mint_open(buyer, collection, *serial, proof, item, inputs, change, nonce)
                })
            }
            Op::RegisterBridged { program, network, origin, decimals } => one(if *program != program::bridge_register() {
                Err("wrong program for a bridged asset")
            } else {
                self.apply_register_bridged(network, origin, *decimals)
            }),
            Op::DepositCommitted { program, asset, gross, external_ref, commitment } => one(if *program != program::deposit() {
                Err("wrong program for a deposit")
            } else {
                self.apply_deposit_committed(asset, *gross, external_ref, commitment)
            }),
            Op::ClaimDeposit { owner, program, secret, asset, output } => one(if *program != program::claim() {
                Err("wrong program for a claim")
            } else {
                self.apply_claim_deposit(owner, secret, asset, output)
            }),
            Op::SetVoucherCap { program, collection, cap } => one(if *program != program::sale_vouchers() {
                Err("wrong program for burn tickets")
            } else {
                self.apply_set_voucher_cap(collection, *cap)
            }),
            Op::PostVoucher { program, collection, burn, commitment, count } => one(if *program != program::sale_vouchers() {
                Err("wrong program for burn tickets")
            } else {
                self.apply_post_voucher(collection, burn, commitment, *count)
            }),
            Op::ClaimVoucherItems { buyer, program, collection, secret, k, serials, proof, items, inputs, change } => {
                one(if *program != program::sale_claim() {
                    Err("wrong program for a claim")
                } else {
                    self.apply_claim_voucher_items(buyer, collection, secret, *k, serials, proof, items, inputs, change)
                })
            }
            Op::LendList { program, asset, max_ltv_bps, liquidation_bps, penalty_bps } => one(if *program != program::lend_list() {
                Err("wrong program for a lending listing")
            } else {
                self.apply_lend_list(asset, zyn_lend::Risk { max_ltv_bps: *max_ltv_bps, liquidation_bps: *liquidation_bps, penalty_bps: *penalty_bps })
            }),
            Op::LendPrices { program, at, prices } => one(if *program != program::lend_prices() {
                Err("wrong program for lending prices")
            } else {
                self.apply_lend_prices(*at, prices)
            }),
            Op::LendSupply { lender, program, asset, amount, nonce, inputs, change } => one(if *program != program::lend_supply() {
                Err("wrong program for lending")
            } else {
                self.apply_lend_supply(lender, asset, *amount, nonce, inputs, change)
            }),
            Op::LendWithdraw { lender, program, asset, shares, nonce, inputs, change } => one(if *program != program::lend_withdraw() {
                Err("wrong program for a lending withdrawal")
            } else {
                self.apply_lend_withdraw(lender, asset, *shares, nonce, inputs, change)
            }),
            Op::LendOpen { borrower, program, collateral_asset, collateral, borrow_asset, amount, nonce, inputs, change } => one(if *program != program::lend_open() {
                Err("wrong program for a borrow")
            } else {
                self.apply_lend_open(borrower, collateral_asset, *collateral, borrow_asset, *amount, nonce, inputs, change)
            }),
            Op::LendRepay { borrower, program, position, amount, nonce, inputs, change } => one(if *program != program::lend_repay() {
                Err("wrong program for a repayment")
            } else {
                self.apply_lend_repay(borrower, position, *amount, nonce, inputs, change)
            }),
            Op::LendAddCollateral { borrower, program, position, amount, inputs, change } => one(if *program != program::lend_collateral() {
                Err("wrong program for collateral")
            } else {
                self.apply_lend_add_collateral(borrower, position, *amount, inputs, change)
            }),
            Op::LendRemoveCollateral { borrower, program, position, amount, nonce } => one(if *program != program::lend_collateral() {
                Err("wrong program for collateral")
            } else {
                self.apply_lend_remove_collateral(borrower, position, *amount, nonce)
            }),
            Op::LendLimits { program, asset, collateral_cap, borrow_cap, min_borrow_usd } => one(if *program != program::lend_list() {
                Err("wrong program for lending limits")
            } else {
                self.apply_lend_limits(asset, lend::Limits { collateral_cap: *collateral_cap, borrow_cap: *borrow_cap, min_borrow_usd: *min_borrow_usd })
            }),
            Op::LendBid { bidder, program, lot, pay, nonce, inputs, change } => one(if *program != program::lend_bid() {
                Err("wrong program for a bid")
            } else {
                self.apply_lend_bid(bidder, lot, *pay, nonce, inputs, change)
            }),
            Op::LendRules { program, version } => one(if *program != program::lend_list() {
                Err("wrong program for lending rules")
            } else {
                self.apply_lend_rules(*version)
            }),
            Op::LendTime { program, at } => one(if *program != program::lend_prices() {
                Err("wrong program for the lending clock")
            } else {
                self.apply_lend_time(*at)
            }),
            Op::LendInsure { donor, program, asset, amount, inputs, change } => one(if *program != program::lend_insure() {
                Err("wrong program for insurance")
            } else {
                self.apply_lend_insure(donor, asset, *amount, inputs, change)
            }),
            Op::Seal => {
                // Batches clear and sales settle inside the epoch they belong
                // to, so the root this seal commits already includes them.
                self.settle_market();
                let state_root = self.state_root();
                let cp = Checkpoint {
                    chain_id: self.chain_id,
                    epoch: self.epoch,
                    parent_root: self.parent_root,
                    state_root,
                    intent_root: self.intent_acc,
                    seq: self.seq,
                    intents: self.epoch_intents,
                    gross_volume: Fixed::ZERO,
                };
                self.parent_root = state_root;
                self.epoch += 1;
                self.intent_acc = [0u8; 32];
                self.epoch_intents = 0;
                vec![Receipt::Sealed(cp)]
            }
        }
    }

    /// Section 1 is the accounts section because the specification pins it
    /// there. Commitments and nullifiers are appended after the sections that
    /// already existed, which is how `collection.rs` was added without
    /// disturbing roots anyone had already committed to.
    fn sections(&self) -> Vec<Hash> {
        let mut s = vec![
            self.header_leaf(),
            merkle_root(&self.account_leaves()),
            self.commitments.root(),
            self.nullifiers.root(),
            self.supply_leaf(),
            self.offers_leaf(),
            merkle_root(&self.live_commitments()),
        ];
        // Only once there is a market: a chain that never used one keeps the
        // roots it already anchored.
        if !self.market.is_empty() {
            s.push(self.market.leaf());
        }
        s
    }

    fn is_seal(intent: &Op) -> bool {
        matches!(intent, Op::Seal)
    }

    fn seal_intent() -> Op {
        Op::Seal
    }

    fn sealed(receipts: &[Receipt]) -> Option<Checkpoint> {
        receipts.iter().find_map(|r| match r {
            Receipt::Sealed(cp) => Some(*cp),
            _ => None,
        })
    }

    fn as_sealed(&self, cp: &Checkpoint) -> Option<Self> {
        if cp.chain_id != self.chain_id {
            return None;
        }
        let mut s = self.clone();
        s.epoch = cp.epoch;
        s.seq = cp.seq;
        s.parent_root = cp.parent_root;
        s.intent_acc = cp.intent_root;
        s.epoch_intents = cp.intents;
        (s.state_root() == cp.state_root).then_some(s)
    }

    fn rejected(r: &Receipt) -> bool {
        matches!(r, Receipt::Refused(_))
    }

    fn unprovable(_: &Receipt) -> bool {
        false
    }

    fn intent_authorities(intent: &Op) -> Vec<AccountId> {
        match intent {
            Op::Spend { spender, .. } => vec![*spender],
            Op::PlaceOffer { seller, .. } | Op::CancelOffer { seller, .. } | Op::List { seller, .. } => vec![*seller],
            // Only the buyer signs a claim. The seller's consent was given
            // when they listed, and requiring it again would make every sale
            // need them online (§132.2).
            Op::TakeOffer { buyer, .. } => vec![*buyer],
            Op::Withdraw { holder, .. } => vec![*holder],
            Op::Register { account, .. } => vec![*account],
            Op::CreatePool { provider, .. }
            | Op::AddLiquidity { provider, .. }
            | Op::RemoveLiquidity { provider, .. } => vec![*provider],
            Op::Swap { trader, .. } | Op::Route { trader, .. } => vec![*trader],
            Op::Claim { owner, .. } => vec![*owner],
            Op::CreateLaunch { creator, .. } => vec![*creator],
            Op::Commit { buyer, .. } => vec![*buyer],
            Op::Redeem { holder, .. } => vec![*holder],
            Op::CreateCurve { creator, .. } => vec![*creator],
            Op::CurveTrade { trader, .. } => vec![*trader],
            Op::BuyTickets { buyer, .. } | Op::ClaimItems { buyer, .. } | Op::ClaimVoucherItems { buyer, .. } | Op::MintOpen { buyer, .. } => vec![*buyer],
            Op::ClaimDeposit { owner, .. } => vec![*owner],
            Op::LendSupply { lender, .. } | Op::LendWithdraw { lender, .. } => vec![*lender],
            Op::LendOpen { borrower, .. }
            | Op::LendRepay { borrower, .. }
            | Op::LendAddCollateral { borrower, .. }
            | Op::LendRemoveCollateral { borrower, .. } => vec![*borrower],
            Op::LendBid { bidder, .. } => vec![*bidder],
            Op::LendInsure { donor, .. } => vec![*donor],
            // LendList, LendPrices, LendLimits, LendRules and LendTime stay
            // operator-only: no user signs them.
            _ => Vec::new(),
        }
    }

    fn encode_intent(intent: &Op) -> Vec<u8> {
        let mut e = Encoder::new();
        match intent {
            Op::Mint {
                asset,
                amount,
                owner,
                nonce,
            } => {
                e.u8(1)
                    .bytes(asset)
                    .fixed(*amount)
                    .bytes(owner)
                    .bytes(nonce);
            }
            Op::Spend {
                spender,
                program,
                inputs,
                outputs,
            } => {
                e.u8(2)
                    .bytes(spender)
                    .bytes(&program.0)
                    .u32(inputs.len() as u32)
                    .u32(outputs.len() as u32);
                for (c, n) in inputs {
                    e.bytes(c).bytes(n);
                }
                for o in outputs {
                    e.bytes(&o.note.asset)
                        .fixed(o.note.amount)
                        .bytes(&o.note.owner)
                        .bytes(&o.note.nonce)
                        .bytes(&o.enc.epk)
                        .u32(o.enc.ciphertext.len() as u32)
                        .bytes(&o.enc.ciphertext);
                }
            }
            Op::PlaceOffer {
                seller,
                program,
                item,
                price_asset,
                price_amount,
                expires_at_epoch,
            } => {
                e.u8(3)
                    .bytes(seller)
                    .bytes(&program.0)
                    .bytes(item)
                    .bytes(price_asset)
                    .fixed(*price_amount)
                    .u64(*expires_at_epoch);
            }
            Op::CancelOffer {
                seller,
                program,
                item,
            } => {
                e.u8(4).bytes(seller).bytes(&program.0).bytes(item);
            }
            Op::TakeOffer {
                buyer,
                program,
                item,
                item_nullifier,
                payment,
                outputs,
            } => {
                e.u8(5)
                    .bytes(buyer)
                    .bytes(&program.0)
                    .bytes(item)
                    .bytes(item_nullifier)
                    .u32(payment.len() as u32)
                    .u32(outputs.len() as u32);
                for (c, n) in payment {
                    e.bytes(c).bytes(n);
                }
                for o in outputs {
                    e.bytes(&o.note.asset)
                        .fixed(o.note.amount)
                        .bytes(&o.note.owner)
                        .bytes(&o.note.nonce)
                        .bytes(&o.enc.epk)
                        .u32(o.enc.ciphertext.len() as u32)
                        .bytes(&o.enc.ciphertext);
                }
            }
            Op::Deposit {
                program,
                asset,
                amount,
                owner,
                nonce,
                enc,
                external_ref,
            } => {
                e.u8(6)
                    .bytes(&program.0)
                    .bytes(asset)
                    .fixed(*amount)
                    .bytes(owner)
                    .bytes(nonce)
                    .bytes(external_ref)
                    .bytes(&enc.epk)
                    .u32(enc.ciphertext.len() as u32)
                    .bytes(&enc.ciphertext);
            }
            Op::Withdraw {
                holder,
                program,
                inputs,
                asset,
                amount,
                destination,
                outputs,
            } => {
                e.u8(7)
                    .bytes(holder)
                    .bytes(&program.0)
                    .bytes(asset)
                    .fixed(*amount)
                    .bytes(destination)
                    .u32(inputs.len() as u32)
                    .u32(outputs.len() as u32);
                for (c, n) in inputs {
                    e.bytes(c).bytes(n);
                }
                for o in outputs {
                    e.bytes(&o.note.asset)
                        .fixed(o.note.amount)
                        .bytes(&o.note.owner)
                        .bytes(&o.note.nonce)
                        .bytes(&o.enc.epk)
                        .u32(o.enc.ciphertext.len() as u32)
                        .bytes(&o.enc.ciphertext);
                }
            }
            Op::Register {
                account,
                program,
                viewing,
            } => {
                e.u8(8)
                    .bytes(account)
                    .bytes(&program.0)
                    .bytes(viewing);
            }
            Op::SettleExit {
                program,
                index,
                txid,
            } => {
                e.u8(9).bytes(&program.0).u32(*index).bytes(txid);
            }
            Op::CreatePool {
                provider,
                program,
                asset_a,
                asset_b,
                amount_a,
                amount_b,
                fee_bps,
                inputs,
                change,
                nonce,
            } => {
                e.u8(10)
                    .bytes(provider)
                    .bytes(&program.0)
                    .bytes(asset_a)
                    .bytes(asset_b)
                    .fixed(*amount_a)
                    .fixed(*amount_b)
                    .u32(*fee_bps)
                    .bytes(nonce);
                wire::inputs(&mut e, inputs);
                wire::outputs(&mut e, change);
            }
            Op::AddLiquidity {
                provider,
                program,
                pool,
                amount_a,
                amount_b,
                min_shares,
                inputs,
                change,
                nonce,
            } => {
                e.u8(11)
                    .bytes(provider)
                    .bytes(&program.0)
                    .bytes(pool)
                    .fixed(*amount_a)
                    .fixed(*amount_b)
                    .fixed(*min_shares)
                    .bytes(nonce);
                wire::inputs(&mut e, inputs);
                wire::outputs(&mut e, change);
            }
            Op::RemoveLiquidity {
                provider,
                program,
                pool,
                shares,
                min_a,
                min_b,
                inputs,
                change,
                nonce,
            } => {
                e.u8(12)
                    .bytes(provider)
                    .bytes(&program.0)
                    .bytes(pool)
                    .fixed(*shares)
                    .fixed(*min_a)
                    .fixed(*min_b)
                    .bytes(nonce);
                wire::inputs(&mut e, inputs);
                wire::outputs(&mut e, change);
            }
            Op::Swap {
                trader,
                program,
                pool,
                sell_a,
                amount_in,
                min_out,
                inputs,
                change,
                nonce,
            } => {
                e.u8(13)
                    .bytes(trader)
                    .bytes(&program.0)
                    .bytes(pool)
                    .bool(*sell_a)
                    .fixed(*amount_in)
                    .fixed(*min_out)
                    .bytes(nonce);
                wire::inputs(&mut e, inputs);
                wire::outputs(&mut e, change);
            }
            Op::Claim {
                owner,
                program,
                claims,
                outputs,
            } => {
                e.u8(14).bytes(owner).bytes(&program.0).u32(claims.len() as u32);
                for c in claims {
                    e.bytes(c);
                }
                wire::outputs(&mut e, outputs);
            }
            Op::CreateLaunch {
                creator,
                program,
                nonce,
                quote,
                total_supply,
                sale_supply,
                pool_supply,
                soft_cap,
                hard_cap,
                pool_bps,
                fee_bps,
                closes_at_epoch,
                meta,
            } => {
                e.u8(15)
                    .bytes(creator)
                    .bytes(&program.0)
                    .bytes(nonce)
                    .bytes(quote)
                    .fixed(*total_supply)
                    .fixed(*sale_supply)
                    .fixed(*pool_supply)
                    .fixed(*soft_cap)
                    .fixed(*hard_cap)
                    .u32(*pool_bps)
                    .u32(*fee_bps)
                    .u64(*closes_at_epoch)
                    .bytes(meta);
            }
            Op::Commit {
                buyer,
                program,
                launch,
                amount,
                inputs,
                change,
                nonce,
            } => {
                e.u8(16)
                    .bytes(buyer)
                    .bytes(&program.0)
                    .bytes(launch)
                    .fixed(*amount)
                    .bytes(nonce);
                wire::inputs(&mut e, inputs);
                wire::outputs(&mut e, change);
            }
            Op::SetFees { program, bridge_bps } => {
                e.u8(17).bytes(&program.0).u32(*bridge_bps);
            }
            Op::RegisterCollection { program, asset, creator, quote, fee_bps, cap } => {
                e.u8(18).bytes(&program.0).bytes(asset).bytes(creator).bytes(quote).u32(*fee_bps).u32(*cap);
            }
            Op::DepositWithFee { program, asset, gross, amount, owner, nonce, enc, external_ref } => {
                e.u8(19)
                    .bytes(&program.0)
                    .bytes(asset)
                    .fixed(*gross)
                    .fixed(*amount)
                    .bytes(owner)
                    .bytes(nonce)
                    .bytes(external_ref)
                    .bytes(&enc.epk)
                    .u32(enc.ciphertext.len() as u32)
                    .bytes(&enc.ciphertext);
            }
            Op::Redeem { holder, program, asset, inputs } => {
                e.u8(20).bytes(holder).bytes(&program.0).bytes(asset);
                wire::inputs(&mut e, inputs);
            }
            Op::List { seller, program, item, item_nullifier, price_asset, price_amount, expires_at_epoch, nonce } => {
                e.u8(21)
                    .bytes(seller)
                    .bytes(&program.0)
                    .bytes(item)
                    .bytes(item_nullifier)
                    .bytes(price_asset)
                    .fixed(*price_amount)
                    .u64(*expires_at_epoch)
                    .bytes(nonce);
            }
            Op::MintItem { program, collection, serial, owner, nonce, enc } => {
                e.u8(22)
                    .bytes(&program.0)
                    .bytes(collection)
                    .u32(*serial)
                    .bytes(owner)
                    .bytes(nonce)
                    .bytes(&enc.epk)
                    .u32(enc.ciphertext.len() as u32)
                    .bytes(&enc.ciphertext);
            }
            Op::CreateCurve { creator, program, nonce, quote, meta, inputs, change } => {
                e.u8(23).bytes(creator).bytes(&program.0).bytes(nonce).bytes(quote).bytes(meta);
                wire::inputs(&mut e, inputs);
                wire::outputs(&mut e, change);
            }
            Op::CurveTrade { trader, program, token, buy, amount_in, min_out, inputs, change, nonce } => {
                e.u8(24)
                    .bytes(trader)
                    .bytes(&program.0)
                    .bytes(token)
                    .bool(*buy)
                    .fixed(*amount_in)
                    .fixed(*min_out)
                    .bytes(nonce);
                wire::inputs(&mut e, inputs);
                wire::outputs(&mut e, change);
            }
            Op::SetCurveTerms { program, fee_bps, bond } => {
                e.u8(25).bytes(&program.0).u32(*fee_bps).fixed(*bond);
            }
            Op::GateSale { program, collection, base } => {
                e.u8(54).bytes(&program.0).bytes(collection).u32(*base);
            }
            Op::AllowBuyers { program, collection, keys } => {
                e.u8(55).bytes(&program.0).bytes(collection).u32(keys.len() as u32);
                for k in keys {
                    e.bytes(k);
                }
            }
            Op::Route { trader, program, hops, amount_in, min_out, inputs, change, nonce } => {
                e.u8(53).bytes(trader).bytes(&program.0);
                route::put_hops(&mut e, hops);
                e.fixed(*amount_in).fixed(*min_out).bytes(nonce);
                wire::inputs(&mut e, inputs);
                wire::outputs(&mut e, change);
            }
            Op::SetCurveQuote { program, quote, graduation_mc, bond } => {
                e.u8(52).bytes(&program.0).bytes(quote).fixed(*graduation_mc).fixed(*bond);
            }
            Op::SetSale { program, collection, ticket_price, claim_price, open_price, treasury_bps } => {
                e.u8(26).bytes(&program.0).bytes(collection).fixed(*ticket_price).fixed(*claim_price).fixed(*open_price).u32(*treasury_bps);
            }
            Op::BuyTickets { buyer, program, collection, count, nonce, inputs, change } => {
                e.u8(27).bytes(buyer).bytes(&program.0).bytes(collection).u32(*count).bytes(nonce);
                wire::inputs(&mut e, inputs);
                wire::outputs(&mut e, change);
            }
            Op::CloseSale { program, collection, zcash_height, zcash_hash } => {
                e.u8(28).bytes(&program.0).bytes(collection).u32(*zcash_height).bytes(zcash_hash);
            }
            Op::PostDraw { program, collection, root } => {
                e.u8(29).bytes(&program.0).bytes(collection).bytes(root);
            }
            Op::ClaimItems { buyer, program, collection, nonce, k, serials, proof, items, inputs, change } => {
                e.u8(30).bytes(buyer).bytes(&program.0).bytes(collection).bytes(nonce).u32(*k).u32(serials.len() as u32);
                for s in serials {
                    e.u32(*s);
                }
                wire::proof(&mut e, proof);
                wire::outputs(&mut e, items);
                wire::inputs(&mut e, inputs);
                wire::outputs(&mut e, change);
            }
            Op::PostOpenMint { program, collection, root, count } => {
                e.u8(31).bytes(&program.0).bytes(collection).bytes(root).u32(*count);
            }
            Op::MintOpen { buyer, program, collection, serial, proof, item, nonce, inputs, change } => {
                e.u8(32).bytes(buyer).bytes(&program.0).bytes(collection).u32(*serial);
                wire::proof(&mut e, proof);
                wire::outputs(&mut e, std::slice::from_ref(item));
                e.bytes(nonce);
                wire::inputs(&mut e, inputs);
                wire::outputs(&mut e, change);
            }
            Op::RegisterBridged { program, network, origin, decimals } => {
                e.u8(36).bytes(&program.0).u32(network.len() as u32).bytes(network).u32(origin.len() as u32).bytes(origin).u8(*decimals);
            }
            Op::DepositCommitted { program, asset, gross, external_ref, commitment } => {
                e.u8(37).bytes(&program.0).bytes(asset).fixed(*gross).bytes(external_ref).bytes(commitment);
            }
            Op::ClaimDeposit { owner, program, secret, asset, output } => {
                e.u8(38).bytes(owner).bytes(&program.0).bytes(secret).bytes(asset);
                wire::outputs(&mut e, std::slice::from_ref(output));
            }
            Op::SetVoucherCap { program, collection, cap } => {
                e.u8(33).bytes(&program.0).bytes(collection).u32(*cap);
            }
            Op::PostVoucher { program, collection, burn, commitment, count } => {
                e.u8(34).bytes(&program.0).bytes(collection).bytes(burn).bytes(commitment).u32(*count);
            }
            Op::ClaimVoucherItems { buyer, program, collection, secret, k, serials, proof, items, inputs, change } => {
                e.u8(35).bytes(buyer).bytes(&program.0).bytes(collection).bytes(secret).u32(*k).u32(serials.len() as u32);
                for s in serials {
                    e.u32(*s);
                }
                wire::proof(&mut e, proof);
                wire::outputs(&mut e, items);
                wire::inputs(&mut e, inputs);
                wire::outputs(&mut e, change);
            }
            Op::LendList { program, asset, max_ltv_bps, liquidation_bps, penalty_bps } => {
                e.u8(39).bytes(&program.0).bytes(asset).u32(*max_ltv_bps).u32(*liquidation_bps).u32(*penalty_bps);
            }
            Op::LendPrices { program, at, prices } => {
                e.u8(40).bytes(&program.0).u64(*at).u32(prices.len() as u32);
                for (a, p) in prices {
                    e.bytes(a).fixed(*p);
                }
            }
            Op::LendSupply { lender, program, asset, amount, nonce, inputs, change } => {
                e.u8(41).bytes(lender).bytes(&program.0).bytes(asset).fixed(*amount).bytes(nonce);
                wire::inputs(&mut e, inputs);
                wire::outputs(&mut e, change);
            }
            Op::LendWithdraw { lender, program, asset, shares, nonce, inputs, change } => {
                e.u8(42).bytes(lender).bytes(&program.0).bytes(asset).fixed(*shares).bytes(nonce);
                wire::inputs(&mut e, inputs);
                wire::outputs(&mut e, change);
            }
            Op::LendOpen { borrower, program, collateral_asset, collateral, borrow_asset, amount, nonce, inputs, change } => {
                e.u8(43)
                    .bytes(borrower)
                    .bytes(&program.0)
                    .bytes(collateral_asset)
                    .fixed(*collateral)
                    .bytes(borrow_asset)
                    .fixed(*amount)
                    .bytes(nonce);
                wire::inputs(&mut e, inputs);
                wire::outputs(&mut e, change);
            }
            Op::LendRepay { borrower, program, position, amount, nonce, inputs, change } => {
                e.u8(44).bytes(borrower).bytes(&program.0).bytes(position).fixed(*amount).bytes(nonce);
                wire::inputs(&mut e, inputs);
                wire::outputs(&mut e, change);
            }
            Op::LendAddCollateral { borrower, program, position, amount, inputs, change } => {
                e.u8(45).bytes(borrower).bytes(&program.0).bytes(position).fixed(*amount);
                wire::inputs(&mut e, inputs);
                wire::outputs(&mut e, change);
            }
            Op::LendRemoveCollateral { borrower, program, position, amount, nonce } => {
                e.u8(46).bytes(borrower).bytes(&program.0).bytes(position).fixed(*amount).bytes(nonce);
            }
            Op::LendLimits { program, asset, collateral_cap, borrow_cap, min_borrow_usd } => {
                e.u8(48).bytes(&program.0).bytes(asset).fixed(*collateral_cap).fixed(*borrow_cap).fixed(*min_borrow_usd);
            }
            Op::LendBid { bidder, program, lot, pay, nonce, inputs, change } => {
                e.u8(47).bytes(bidder).bytes(&program.0).bytes(lot).fixed(*pay).bytes(nonce);
                wire::inputs(&mut e, inputs);
                wire::outputs(&mut e, change);
            }
            Op::LendRules { program, version } => {
                e.u8(49).bytes(&program.0).u32(*version);
            }
            Op::LendTime { program, at } => {
                e.u8(50).bytes(&program.0).u64(*at);
            }
            Op::LendInsure { donor, program, asset, amount, inputs, change } => {
                e.u8(51).bytes(donor).bytes(&program.0).bytes(asset).fixed(*amount);
                wire::inputs(&mut e, inputs);
                wire::outputs(&mut e, change);
            }
            Op::Seal => {
                e.u8(0);
            }
        }
        e.finish().to_vec()
    }

    fn decode_intent(d: &mut Decoder) -> Option<Op> {
        match d.u8().ok()? {
            0 => Some(Op::Seal),
            1 => Some(Op::Mint {
                asset: d.hash().ok()?,
                amount: d.fixed().ok()?,
                owner: d.account().ok()?,
                nonce: d.hash().ok()?,
            }),
            2 => {
                let spender = d.account().ok()?;
                let program = ProgramId(d.hash().ok()?);
                let ins = d.u32().ok()? as usize;
                let outs = d.u32().ok()? as usize;
                let mut inputs = Vec::with_capacity(ins.min(1024));
                for _ in 0..ins {
                    inputs.push((d.hash().ok()?, d.hash().ok()?));
                }
                let mut outputs = Vec::with_capacity(outs.min(1024));
                for _ in 0..outs {
                    let note = Note {
                        asset: d.hash().ok()?,
                        amount: d.fixed().ok()?,
                        owner: d.account().ok()?,
                        nonce: d.hash().ok()?,
                    };
                    let epk = d.hash().ok()?;
                    let n = d.u32().ok()? as usize;
                    let ciphertext = d.take_bytes(n).ok()?.to_vec();
                    outputs.push(Output {
                        note,
                        enc: EncryptedNote { epk, ciphertext },
                    });
                }
                Some(Op::Spend {
                    spender,
                    program,
                    inputs,
                    outputs,
                })
            }
            3 => Some(Op::PlaceOffer {
                seller: d.account().ok()?,
                program: ProgramId(d.hash().ok()?),
                item: d.hash().ok()?,
                price_asset: d.hash().ok()?,
                price_amount: d.fixed().ok()?,
                expires_at_epoch: d.u64().ok()?,
            }),
            4 => Some(Op::CancelOffer {
                seller: d.account().ok()?,
                program: ProgramId(d.hash().ok()?),
                item: d.hash().ok()?,
            }),
            5 => {
                let buyer = d.account().ok()?;
                let program = ProgramId(d.hash().ok()?);
                let item = d.hash().ok()?;
                let item_nullifier = d.hash().ok()?;
                let pays = d.u32().ok()? as usize;
                let outs = d.u32().ok()? as usize;
                let mut payment = Vec::with_capacity(pays.min(1024));
                for _ in 0..pays {
                    payment.push((d.hash().ok()?, d.hash().ok()?));
                }
                let mut outputs = Vec::with_capacity(outs.min(1024));
                for _ in 0..outs {
                    let note = Note {
                        asset: d.hash().ok()?,
                        amount: d.fixed().ok()?,
                        owner: d.account().ok()?,
                        nonce: d.hash().ok()?,
                    };
                    let epk = d.hash().ok()?;
                    let n = d.u32().ok()? as usize;
                    let ciphertext = d.take_bytes(n).ok()?.to_vec();
                    outputs.push(Output {
                        note,
                        enc: EncryptedNote { epk, ciphertext },
                    });
                }
                Some(Op::TakeOffer {
                    buyer,
                    program,
                    item,
                    item_nullifier,
                    payment,
                    outputs,
                })
            }
            6 => {
                let program = ProgramId(d.hash().ok()?);
                let asset = d.hash().ok()?;
                let amount = d.fixed().ok()?;
                let owner = d.account().ok()?;
                let nonce = d.hash().ok()?;
                let external_ref = d.hash().ok()?;
                let epk = d.hash().ok()?;
                let n = d.u32().ok()? as usize;
                let ciphertext = d.take_bytes(n).ok()?.to_vec();
                Some(Op::Deposit {
                    program,
                    asset,
                    amount,
                    owner,
                    nonce,
                    enc: EncryptedNote { epk, ciphertext },
                    external_ref,
                })
            }
            7 => {
                let holder = d.account().ok()?;
                let program = ProgramId(d.hash().ok()?);
                let asset = d.hash().ok()?;
                let amount = d.fixed().ok()?;
                let destination = d.hash().ok()?;
                let ins = d.u32().ok()? as usize;
                let outs = d.u32().ok()? as usize;
                let mut inputs = Vec::with_capacity(ins.min(1024));
                for _ in 0..ins {
                    inputs.push((d.hash().ok()?, d.hash().ok()?));
                }
                let mut outputs = Vec::with_capacity(outs.min(1024));
                for _ in 0..outs {
                    let note = Note {
                        asset: d.hash().ok()?,
                        amount: d.fixed().ok()?,
                        owner: d.account().ok()?,
                        nonce: d.hash().ok()?,
                    };
                    let epk = d.hash().ok()?;
                    let n = d.u32().ok()? as usize;
                    let ciphertext = d.take_bytes(n).ok()?.to_vec();
                    outputs.push(Output { note, enc: EncryptedNote { epk, ciphertext } });
                }
                Some(Op::Withdraw {
                    holder,
                    program,
                    inputs,
                    asset,
                    amount,
                    destination,
                    outputs,
                })
            }
            8 => Some(Op::Register {
                account: d.account().ok()?,
                program: ProgramId(d.hash().ok()?),
                viewing: d.hash().ok()?,
            }),
            9 => Some(Op::SettleExit {
                program: ProgramId(d.hash().ok()?),
                index: d.u32().ok()?,
                txid: d.hash().ok()?,
            }),
            10 => Some(Op::CreatePool {
                provider: d.account().ok()?,
                program: ProgramId(d.hash().ok()?),
                asset_a: d.hash().ok()?,
                asset_b: d.hash().ok()?,
                amount_a: d.fixed().ok()?,
                amount_b: d.fixed().ok()?,
                fee_bps: d.u32().ok()?,
                nonce: d.hash().ok()?,
                inputs: wire::read_inputs(d)?,
                change: wire::read_outputs(d)?,
            }),
            11 => Some(Op::AddLiquidity {
                provider: d.account().ok()?,
                program: ProgramId(d.hash().ok()?),
                pool: d.hash().ok()?,
                amount_a: d.fixed().ok()?,
                amount_b: d.fixed().ok()?,
                min_shares: d.fixed().ok()?,
                nonce: d.hash().ok()?,
                inputs: wire::read_inputs(d)?,
                change: wire::read_outputs(d)?,
            }),
            12 => Some(Op::RemoveLiquidity {
                provider: d.account().ok()?,
                program: ProgramId(d.hash().ok()?),
                pool: d.hash().ok()?,
                shares: d.fixed().ok()?,
                min_a: d.fixed().ok()?,
                min_b: d.fixed().ok()?,
                nonce: d.hash().ok()?,
                inputs: wire::read_inputs(d)?,
                change: wire::read_outputs(d)?,
            }),
            13 => Some(Op::Swap {
                trader: d.account().ok()?,
                program: ProgramId(d.hash().ok()?),
                pool: d.hash().ok()?,
                sell_a: match d.u8().ok()? {
                    0 => false,
                    1 => true,
                    _ => return None,
                },
                amount_in: d.fixed().ok()?,
                min_out: d.fixed().ok()?,
                nonce: d.hash().ok()?,
                inputs: wire::read_inputs(d)?,
                change: wire::read_outputs(d)?,
            }),
            14 => {
                let owner = d.account().ok()?;
                let program = ProgramId(d.hash().ok()?);
                let n = d.u32().ok()? as usize;
                let mut claims = Vec::with_capacity(n.min(1024));
                for _ in 0..n {
                    claims.push(d.hash().ok()?);
                }
                Some(Op::Claim {
                    owner,
                    program,
                    claims,
                    outputs: wire::read_outputs(d)?,
                })
            }
            15 => Some(Op::CreateLaunch {
                creator: d.account().ok()?,
                program: ProgramId(d.hash().ok()?),
                nonce: d.hash().ok()?,
                quote: d.hash().ok()?,
                total_supply: d.fixed().ok()?,
                sale_supply: d.fixed().ok()?,
                pool_supply: d.fixed().ok()?,
                soft_cap: d.fixed().ok()?,
                hard_cap: d.fixed().ok()?,
                pool_bps: d.u32().ok()?,
                fee_bps: d.u32().ok()?,
                closes_at_epoch: d.u64().ok()?,
                meta: d.hash().ok()?,
            }),
            16 => Some(Op::Commit {
                buyer: d.account().ok()?,
                program: ProgramId(d.hash().ok()?),
                launch: d.hash().ok()?,
                amount: d.fixed().ok()?,
                nonce: d.hash().ok()?,
                inputs: wire::read_inputs(d)?,
                change: wire::read_outputs(d)?,
            }),
            17 => Some(Op::SetFees {
                program: ProgramId(d.hash().ok()?),
                bridge_bps: d.u32().ok()?,
            }),
            18 => Some(Op::RegisterCollection {
                program: ProgramId(d.hash().ok()?),
                asset: d.hash().ok()?,
                creator: d.account().ok()?,
                quote: d.hash().ok()?,
                fee_bps: d.u32().ok()?,
                cap: d.u32().ok()?,
            }),
            19 => {
                let program = ProgramId(d.hash().ok()?);
                let asset = d.hash().ok()?;
                let gross = d.fixed().ok()?;
                let amount = d.fixed().ok()?;
                let owner = d.account().ok()?;
                let nonce = d.hash().ok()?;
                let external_ref = d.hash().ok()?;
                let epk = d.hash().ok()?;
                let n = d.u32().ok()? as usize;
                let ciphertext = d.take_bytes(n).ok()?.to_vec();
                Some(Op::DepositWithFee {
                    program,
                    asset,
                    gross,
                    amount,
                    owner,
                    nonce,
                    enc: EncryptedNote { epk, ciphertext },
                    external_ref,
                })
            }
            20 => Some(Op::Redeem {
                holder: d.account().ok()?,
                program: ProgramId(d.hash().ok()?),
                asset: d.hash().ok()?,
                inputs: wire::read_inputs(d)?,
            }),
            21 => Some(Op::List {
                seller: d.account().ok()?,
                program: ProgramId(d.hash().ok()?),
                item: d.hash().ok()?,
                item_nullifier: d.hash().ok()?,
                price_asset: d.hash().ok()?,
                price_amount: d.fixed().ok()?,
                expires_at_epoch: d.u64().ok()?,
                nonce: d.hash().ok()?,
            }),
            22 => {
                let program = ProgramId(d.hash().ok()?);
                let collection = d.hash().ok()?;
                let serial = d.u32().ok()?;
                let owner = d.account().ok()?;
                let nonce = d.hash().ok()?;
                let epk = d.hash().ok()?;
                let n = d.u32().ok()? as usize;
                let ciphertext = d.take_bytes(n).ok()?.to_vec();
                Some(Op::MintItem { program, collection, serial, owner, nonce, enc: EncryptedNote { epk, ciphertext } })
            }
            23 => Some(Op::CreateCurve {
                creator: d.account().ok()?,
                program: ProgramId(d.hash().ok()?),
                nonce: d.hash().ok()?,
                quote: d.hash().ok()?,
                meta: d.hash().ok()?,
                inputs: wire::read_inputs(d)?,
                change: wire::read_outputs(d)?,
            }),
            24 => Some(Op::CurveTrade {
                trader: d.account().ok()?,
                program: ProgramId(d.hash().ok()?),
                token: d.hash().ok()?,
                buy: match d.u8().ok()? {
                    0 => false,
                    1 => true,
                    _ => return None,
                },
                amount_in: d.fixed().ok()?,
                min_out: d.fixed().ok()?,
                nonce: d.hash().ok()?,
                inputs: wire::read_inputs(d)?,
                change: wire::read_outputs(d)?,
            }),
            25 => Some(Op::SetCurveTerms {
                program: ProgramId(d.hash().ok()?),
                fee_bps: d.u32().ok()?,
                bond: d.fixed().ok()?,
            }),
            26 => Some(Op::SetSale {
                program: ProgramId(d.hash().ok()?),
                collection: d.hash().ok()?,
                ticket_price: d.fixed().ok()?,
                claim_price: d.fixed().ok()?,
                open_price: d.fixed().ok()?,
                treasury_bps: d.u32().ok()?,
            }),
            27 => Some(Op::BuyTickets {
                buyer: d.account().ok()?,
                program: ProgramId(d.hash().ok()?),
                collection: d.hash().ok()?,
                count: d.u32().ok()?,
                nonce: d.hash().ok()?,
                inputs: wire::read_inputs(d)?,
                change: wire::read_outputs(d)?,
            }),
            28 => Some(Op::CloseSale {
                program: ProgramId(d.hash().ok()?),
                collection: d.hash().ok()?,
                zcash_height: d.u32().ok()?,
                zcash_hash: d.hash().ok()?,
            }),
            29 => Some(Op::PostDraw { program: ProgramId(d.hash().ok()?), collection: d.hash().ok()?, root: d.hash().ok()? }),
            30 => {
                let buyer = d.account().ok()?;
                let program = ProgramId(d.hash().ok()?);
                let collection = d.hash().ok()?;
                let nonce = d.hash().ok()?;
                let k = d.u32().ok()?;
                let n = d.u32().ok()? as usize;
                if n > sale::MAX_ITEMS_PER_TICKET {
                    return None;
                }
                let mut serials = Vec::with_capacity(n);
                for _ in 0..n {
                    serials.push(d.u32().ok()?);
                }
                let proof = wire::read_proof(d)?;
                let items = wire::read_outputs(d)?;
                Some(Op::ClaimItems { buyer, program, collection, nonce, k, serials, proof, items, inputs: wire::read_inputs(d)?, change: wire::read_outputs(d)? })
            }
            36 => {
                let program = ProgramId(d.hash().ok()?);
                let n = d.u32().ok()? as usize;
                if n > 64 {
                    return None;
                }
                let network = d.take_bytes(n).ok()?.to_vec();
                let m = d.u32().ok()? as usize;
                if m > 32 {
                    return None;
                }
                let origin = d.take_bytes(m).ok()?.to_vec();
                Some(Op::RegisterBridged { program, network, origin, decimals: d.u8().ok()? })
            }
            37 => Some(Op::DepositCommitted {
                program: ProgramId(d.hash().ok()?),
                asset: d.hash().ok()?,
                gross: d.fixed().ok()?,
                external_ref: d.hash().ok()?,
                commitment: d.hash().ok()?,
            }),
            38 => {
                let owner = d.account().ok()?;
                let program = ProgramId(d.hash().ok()?);
                let secret = d.hash().ok()?;
                let asset = d.hash().ok()?;
                let mut one = wire::read_outputs(d)?;
                if one.len() != 1 {
                    return None;
                }
                Some(Op::ClaimDeposit { owner, program, secret, asset, output: one.remove(0) })
            }
            33 => Some(Op::SetVoucherCap { program: ProgramId(d.hash().ok()?), collection: d.hash().ok()?, cap: d.u32().ok()? }),
            34 => Some(Op::PostVoucher {
                program: ProgramId(d.hash().ok()?),
                collection: d.hash().ok()?,
                burn: d.hash().ok()?,
                commitment: d.hash().ok()?,
                count: d.u32().ok()?,
            }),
            35 => {
                let buyer = d.account().ok()?;
                let program = ProgramId(d.hash().ok()?);
                let collection = d.hash().ok()?;
                let secret = d.hash().ok()?;
                let k = d.u32().ok()?;
                let n = d.u32().ok()? as usize;
                if n > sale::MAX_ITEMS_PER_TICKET {
                    return None;
                }
                let mut serials = Vec::with_capacity(n);
                for _ in 0..n {
                    serials.push(d.u32().ok()?);
                }
                let proof = wire::read_proof(d)?;
                let items = wire::read_outputs(d)?;
                Some(Op::ClaimVoucherItems { buyer, program, collection, secret, k, serials, proof, items, inputs: wire::read_inputs(d)?, change: wire::read_outputs(d)? })
            }
            31 => Some(Op::PostOpenMint {
                program: ProgramId(d.hash().ok()?),
                collection: d.hash().ok()?,
                root: d.hash().ok()?,
                count: d.u32().ok()?,
            }),
            32 => {
                let buyer = d.account().ok()?;
                let program = ProgramId(d.hash().ok()?);
                let collection = d.hash().ok()?;
                let serial = d.u32().ok()?;
                let proof = wire::read_proof(d)?;
                let mut one_item = wire::read_outputs(d)?;
                if one_item.len() != 1 {
                    return None;
                }
                let item = one_item.remove(0);
                Some(Op::MintOpen { buyer, program, collection, serial, proof, item, nonce: d.hash().ok()?, inputs: wire::read_inputs(d)?, change: wire::read_outputs(d)? })
            }
            39 => Some(Op::LendList {
                program: ProgramId(d.hash().ok()?),
                asset: d.hash().ok()?,
                max_ltv_bps: d.u32().ok()?,
                liquidation_bps: d.u32().ok()?,
                penalty_bps: d.u32().ok()?,
            }),
            40 => {
                let program = ProgramId(d.hash().ok()?);
                let at = d.u64().ok()?;
                let n = d.u32().ok()?;
                if n > 64 {
                    return None;
                }
                let mut prices = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    prices.push((d.hash().ok()?, d.fixed().ok()?));
                }
                Some(Op::LendPrices { program, at, prices })
            }
            41 => Some(Op::LendSupply {
                lender: d.account().ok()?,
                program: ProgramId(d.hash().ok()?),
                asset: d.hash().ok()?,
                amount: d.fixed().ok()?,
                nonce: d.hash().ok()?,
                inputs: wire::read_inputs(d)?,
                change: wire::read_outputs(d)?,
            }),
            42 => Some(Op::LendWithdraw {
                lender: d.account().ok()?,
                program: ProgramId(d.hash().ok()?),
                asset: d.hash().ok()?,
                shares: d.fixed().ok()?,
                nonce: d.hash().ok()?,
                inputs: wire::read_inputs(d)?,
                change: wire::read_outputs(d)?,
            }),
            43 => Some(Op::LendOpen {
                borrower: d.account().ok()?,
                program: ProgramId(d.hash().ok()?),
                collateral_asset: d.hash().ok()?,
                collateral: d.fixed().ok()?,
                borrow_asset: d.hash().ok()?,
                amount: d.fixed().ok()?,
                nonce: d.hash().ok()?,
                inputs: wire::read_inputs(d)?,
                change: wire::read_outputs(d)?,
            }),
            44 => Some(Op::LendRepay {
                borrower: d.account().ok()?,
                program: ProgramId(d.hash().ok()?),
                position: d.hash().ok()?,
                amount: d.fixed().ok()?,
                nonce: d.hash().ok()?,
                inputs: wire::read_inputs(d)?,
                change: wire::read_outputs(d)?,
            }),
            45 => Some(Op::LendAddCollateral {
                borrower: d.account().ok()?,
                program: ProgramId(d.hash().ok()?),
                position: d.hash().ok()?,
                amount: d.fixed().ok()?,
                inputs: wire::read_inputs(d)?,
                change: wire::read_outputs(d)?,
            }),
            46 => Some(Op::LendRemoveCollateral {
                borrower: d.account().ok()?,
                program: ProgramId(d.hash().ok()?),
                position: d.hash().ok()?,
                amount: d.fixed().ok()?,
                nonce: d.hash().ok()?,
            }),
            48 => Some(Op::LendLimits {
                program: ProgramId(d.hash().ok()?),
                asset: d.hash().ok()?,
                collateral_cap: d.fixed().ok()?,
                borrow_cap: d.fixed().ok()?,
                min_borrow_usd: d.fixed().ok()?,
            }),
            47 => Some(Op::LendBid {
                bidder: d.account().ok()?,
                program: ProgramId(d.hash().ok()?),
                lot: d.hash().ok()?,
                pay: d.fixed().ok()?,
                nonce: d.hash().ok()?,
                inputs: wire::read_inputs(d)?,
                change: wire::read_outputs(d)?,
            }),
            49 => Some(Op::LendRules { program: ProgramId(d.hash().ok()?), version: d.u32().ok()? }),
            50 => Some(Op::LendTime { program: ProgramId(d.hash().ok()?), at: d.u64().ok()? }),
            54 => Some(Op::GateSale { program: ProgramId(d.hash().ok()?), collection: d.hash().ok()?, base: d.u32().ok()? }),
            55 => Some(Op::AllowBuyers {
                program: ProgramId(d.hash().ok()?),
                collection: d.hash().ok()?,
                keys: {
                    let n = d.u32().ok()? as usize;
                    if n > crate::sale::MAX_ALLOW_PER_OP {
                        return None;
                    }
                    let mut v = Vec::with_capacity(n);
                    for _ in 0..n {
                        v.push(d.hash().ok()?);
                    }
                    v
                },
            }),
            53 => Some(Op::Route {
                trader: d.account().ok()?,
                program: ProgramId(d.hash().ok()?),
                hops: route::read_hops(d)?,
                amount_in: d.fixed().ok()?,
                min_out: d.fixed().ok()?,
                nonce: d.hash().ok()?,
                inputs: wire::read_inputs(d)?,
                change: wire::read_outputs(d)?,
            }),
            52 => Some(Op::SetCurveQuote {
                program: ProgramId(d.hash().ok()?),
                quote: d.hash().ok()?,
                graduation_mc: d.fixed().ok()?,
                bond: d.fixed().ok()?,
            }),
            51 => Some(Op::LendInsure {
                donor: d.account().ok()?,
                program: ProgramId(d.hash().ok()?),
                asset: d.hash().ok()?,
                amount: d.fixed().ok()?,
                inputs: wire::read_inputs(d)?,
                change: wire::read_outputs(d)?,
            }),
            _ => None,
        }
    }

    fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.bytes(b"OBJVM001")
            .u32(self.chain_id)
            .u64(self.epoch)
            .u64(self.seq)
            .u32(self.limits.max_io)
            .bytes(&self.parent_root)
            .bytes(&self.intent_acc)
            .u64(self.epoch_intents)
            .u32(self.accounts.len() as u32);
        for (id, spends) in &self.accounts {
            e.bytes(id).u64(*spends);
        }
        e.u32(self.commitments.len() as u32);
        for c in self.commitments.leaves() {
            e.bytes(c);
        }
        e.u32(self.nullifiers.len() as u32);
        for n in self.nullifiers.spent() {
            e.bytes(n);
        }
        e.u32(self.live.len() as u32);
        for (c, note) in &self.live {
            e.bytes(c)
                .bytes(&note.asset)
                .fixed(note.amount)
                .bytes(&note.owner)
                .bytes(&note.nonce);
        }
        e.u32(self.issued.len() as u32);
        for (asset, amount) in &self.issued {
            e.bytes(asset).fixed(*amount);
        }
        e.u32(self.deposits.len() as u32);
        for r in &self.deposits {
            e.bytes(r);
        }
        e.u32(self.exits.len() as u32);
        for x in &self.exits {
            e.bytes(&x.asset)
                .fixed(x.amount)
                .bytes(&x.destination)
                .u64(x.since_epoch)
                .opt_hash(x.paid_by);
        }
        e.u32(self.viewing.len() as u32);
        for (a, v) in &self.viewing {
            e.bytes(a).bytes(v);
        }
        e.u32(self.offers.len() as u32);
        for (item, o) in &self.offers {
            e.bytes(item)
                .bytes(&o.seller)
                .bytes(&o.price_asset)
                .fixed(o.price_amount)
                .u64(o.expires_at_epoch);
        }
        // Appended only when there is one, so a state saved before the market
        // existed and one that never used it encode identically.
        if !self.market.is_empty() {
            self.market.encode(&mut e);
        }
        e.finish().to_vec()
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        let mut d = Decoder::new(bytes);
        if d.take_bytes(8).ok()? != b"OBJVM001" {
            return None;
        }
        let chain_id = d.u32().ok()?;
        let epoch = d.u64().ok()?;
        let seq = d.u64().ok()?;
        let max_io = d.u32().ok()?;
        let parent_root = d.hash().ok()?;
        let intent_acc = d.hash().ok()?;
        let epoch_intents = d.u64().ok()?;

        let mut accounts = BTreeMap::new();
        for _ in 0..d.u32().ok()? {
            accounts.insert(d.account().ok()?, d.u64().ok()?);
        }
        let mut commitments = Commitments::new();
        for _ in 0..d.u32().ok()? {
            commitments.append(d.hash().ok()?);
        }
        let mut nullifiers = Nullifiers::new();
        for _ in 0..d.u32().ok()? {
            nullifiers.insert(d.hash().ok()?);
        }
        let mut live = BTreeMap::new();
        for _ in 0..d.u32().ok()? {
            let c = d.hash().ok()?;
            live.insert(
                c,
                Note {
                    asset: d.hash().ok()?,
                    amount: d.fixed().ok()?,
                    owner: d.account().ok()?,
                    nonce: d.hash().ok()?,
                },
            );
        }
        let mut issued = BTreeMap::new();
        for _ in 0..d.u32().ok()? {
            issued.insert(d.hash().ok()?, d.fixed().ok()?);
        }
        let mut deposits = BTreeSet::new();
        for _ in 0..d.u32().ok()? {
            deposits.insert(d.hash().ok()?);
        }
        let mut exits = Vec::new();
        for _ in 0..d.u32().ok()? {
            exits.push(PendingExit {
                asset: d.hash().ok()?,
                amount: d.fixed().ok()?,
                destination: d.hash().ok()?,
                since_epoch: d.u64().ok()?,
                paid_by: {
                    let present = d.u8().ok()?;
                    let h = d.hash().ok()?;
                    (present == 1).then_some(h)
                },
            });
        }
        let mut viewing = BTreeMap::new();
        for _ in 0..d.u32().ok()? {
            viewing.insert(d.account().ok()?, d.hash().ok()?);
        }
        let mut offers = BTreeMap::new();
        for _ in 0..d.u32().ok()? {
            let item = d.hash().ok()?;
            offers.insert(
                item,
                Offer {
                    seller: d.account().ok()?,
                    price_asset: d.hash().ok()?,
                    price_amount: d.fixed().ok()?,
                    expires_at_epoch: d.u64().ok()?,
                },
            );
        }
        let market = if d.remaining() > 0 {
            market::Market::decode(&mut d)?
        } else {
            market::Market::default()
        };
        if d.remaining() > 0 {
            return None;
        }
        Some(ObjectVm {
            market,
            chain_id,
            epoch,
            seq,
            limits: Limits { max_io },
            parent_root,
            intent_acc,
            epoch_intents,
            accounts,
            commitments,
            nullifiers,
            live,
            issued,
            offers,
            deposits,
            exits,
            viewing,
        })
    }

    fn account_ids(&self) -> Vec<AccountId> {
        self.accounts.keys().copied().collect()
    }

    fn account_leaves(&self) -> Vec<Hash> {
        self.accounts
            .iter()
            .map(|(id, spends)| {
                Self::account_leaf(id, *spends, self.viewing.get(id))
            })
            .collect()
    }

    fn account_record(&self, id: &AccountId) -> Option<Vec<u8>> {
        let spends = *self.accounts.get(id)?;
        Some(Self::account_record_bytes(id, spends, self.viewing.get(id)))
    }

    fn leaf_of_record(record: &[u8]) -> Hash {
        zyn_vm::commit::hash_leaf(record)
    }

    fn gross_volume(&self) -> Fixed {
        Fixed::ZERO
    }

    /// §130.6 said commitments break this. They do not break it *here*: the
    /// sum runs over live note amounts rather than account balances, which is
    /// the same equality over a different container. It breaks at tier 3, when
    /// there is no readable amount left to sum.
    fn conserved(&self) -> Result<(), &'static str> {
        for (asset, issued) in &self.issued {
            // Live objects, plus what the market holds for somebody: pool
            // reserves, orders in the batch, claims not yet redeemed, sales
            // in escrow.
            let held = self
                .held(asset)
                .and_then(|h| h.0.checked_add(self.market.holdings(asset)?))
                .map(Fixed)
                .ok_or("held overflow")?;
            if held != *issued {
                return Err("live objects do not sum to issued supply");
            }
        }
        Ok(())
    }
}

/// Turning a confirmed Zcash deposit into an object.
///
/// The bridge watches Zcash and produces "this txid paid this much to this
/// account". The object ledger needs more than that: a nonce, so the
/// commitment hides, and a ciphertext, so the depositor can find what was
/// made for them. Neither can come from the deposit itself.
/// Which section of the state root holds what.
///
/// `sections()` returns these in order and the state root is their Merkle
/// root, so an exit proof has to name the index it opens. Positional literals
/// in a proof verifier are how a reader ends up checking a commitment path
/// against the supply root and finding that it fails for the wrong reason.
pub const SECTION_ACCOUNTS: usize = 1;
pub const SECTION_COMMITMENTS: usize = 2;
pub const SECTION_NULLIFIERS: usize = 3;
/// Which objects are unspent.
///
/// Added because absence from the nullifier set is **not** a sound test of
/// unspentness at this tier (§136.5). A spender supplies their own nullifier
/// and the VM cannot check it derives from their key — it does not hold the
/// key, and there is no circuit yet to constrain one. So a spend can publish
/// a nullifier of the spender's choosing, and the object's real nullifier
/// never appears. `live` is the VM's own record and needs no key to trust.
pub const SECTION_LIVE: usize = 6;

/// Getting out when nobody is left to ask.
///
/// The account ledger's escape hatch enumerates every balance and proves the
/// list hashes into the root a Zcash transaction anchored (`zyn-cli record` /
/// `verify-exit`). An object ledger has no enumerable balance to point at, so
/// that proof has no object-model translation and a holder would have nothing
/// to show a custodian if the sequencer went away. Designing the replacement
/// is a precondition of the switch, not a follow-up (A106).
///
/// What a holder must establish is two things, and the second is the hard one:
///
/// 1. **This note existed.** Its commitment is a leaf of the commitments tree,
///    and that tree's root is a section of a root anchored on Zcash.
/// 2. **It was not already spent.** Its nullifier is absent from the spent set
///    at that same root.
///
/// # Why the nullifier key is revealed
///
/// A nullifier is `H(nullifier_key, commitment)`, and the key is the holder's.
/// If the proof merely *stated* its nullifier, a holder could state one that
/// happens to be absent while the note's real nullifier sat in the set —
/// claiming a note they had already spent. The verifier has to derive the
/// nullifier itself, which means it needs the key.
///
/// Revealing it costs privacy and not money: the nullifier key recognises
/// spends, it cannot authorise one. Anyone holding it can link that account's
/// spends to each other, and still cannot move a coin. This is the recovery
/// path — the sequencer is gone and the holder is proving a claim in public —
/// so linkability is already the situation rather than a new loss.
///
/// # Why the whole spent set travels with the proof
///
/// Absence is not provable from a root alone. The verifier recomputes the
/// nullifier root from the set it is given and checks the note's nullifier is
/// not in it, which is unconditional: a forged set produces a different root
/// and fails against the anchor.
///
/// The cost is a proof linear in total spends rather than logarithmic. That is
/// the same trade the account model already makes — its proof carries every
/// record — and it is the right one while the alternative is a compact
/// exclusion proof whose soundness rests on the set being sorted, which a
/// verifier holding only two neighbours cannot check. When the set outgrows
/// this, the fix is to bind each leaf's index into its hash so adjacency
/// becomes provable; until then, carrying the set is honest and small.
pub mod exit {
    use super::*;
    use zyn_vm::commit::{merkle_proof, merkle_root, verify_proof, ProofStep};
    use zyn_vm::object::{commitment, Note};

    /// Why a claim was not believed.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum ExitError {
        /// The note does not hash to the commitment the path opens.
        NoteDoesNotOpenCommitment,
        /// The commitment is not in the tree this root committed to.
        NotInCommitmentTree,
        /// The object is not in the live set: it has been spent.
        NotLive,
        /// The spent set given does not hash to the nullifier root.
        SpentSetDoesNotMatchRoot,
        /// The set is not a set: unsorted, or carrying a duplicate.
        SpentSetMalformed,
        /// The note's nullifier is in the spent set. It is already spent.
        AlreadySpent,
        /// The sections do not hash to the root that was anchored.
        SectionsDoNotOpenAnchor,
        /// Too few sections to contain the ones this proof reads.
        MalformedSections,
        /// A note of no value, or with a zero nonce, is not claimable.
        UnclaimableNote,
        /// The account leaf is not in the accounts tree this root committed
        /// to — the registration being claimed was never made.
        AccountNotRegistered,
        /// The nullifier key presented is not the one this account
        /// registered. Without this check a claimant could present any key,
        /// derive a nullifier absent from the spent set, and be paid for a
        /// note they had already spent.
        NullifierKeyNotThisAccount,
    }

    /// One holder's claim on the vault, checkable by anyone.
    #[derive(Clone, PartialEq, Eq, Debug)]
    pub struct ExitProof {
        /// Opened in full, so the verifier can recompute the commitment
        /// rather than take one on trust.
        pub note: Note,
        /// Where the commitment sits in the commitments tree. That tree
        /// appends and never sorts, so a position is permanent.
        pub position: u64,
        /// Commitment to commitments root: this object was created.
        pub commitment_path: Vec<ProofStep>,
        /// Commitment to live root: and it has not been spent.
        pub live_path: Vec<ProofStep>,
        /// The sections whose Merkle root is the state root.
        pub sections: Vec<Hash>,
        /// The root a Zcash transaction committed to.
        pub anchored: Hash,
    }

    impl ExitProof {
        /// What the vault owes this holder, if the proof stands.
        pub fn claim(&self) -> Result<(Address, Fixed), ExitError> {
            self.verify()?;
            Ok((self.note.asset, self.note.amount))
        }

        /// Check the whole claim. Returns `Ok` only if every step holds.
        pub fn verify(&self) -> Result<(), ExitError> {
            if self.sections.len() <= SECTION_LIVE {
                return Err(ExitError::MalformedSections);
            }
            // The note must open its own commitment. `commitment` refuses a
            // zero amount and a zero nonce, so an unclaimable note is
            // rejected here rather than producing a claim for nothing.
            let c = commitment(&self.note).map_err(|_| ExitError::UnclaimableNote)?;

            // It existed...
            if !verify_proof(c, &self.commitment_path, self.sections[SECTION_COMMITMENTS]) {
                return Err(ExitError::NotInCommitmentTree);
            }
            // ...and it is still unspent. This is the VM's own record of
            // which objects remain, which is why it needs no key to check and
            // why a spender cannot forge their way around it (§136.5).
            if !verify_proof(c, &self.live_path, self.sections[SECTION_LIVE]) {
                return Err(ExitError::NotLive);
            }

            // And all of it has to hang off the root Zcash saw. Without this
            // the proof is internally consistent and describes a chain nobody
            // else agreed to.
            if merkle_root(&self.sections) != self.anchored {
                return Err(ExitError::SectionsDoNotOpenAnchor);
            }
            Ok(())
        }
    }

    /// Build a holder's proof against the current state.
    ///
    /// `anchored` is the root a Zcash transaction committed. It is passed in
    /// rather than read from the VM so that a proof is always built against a
    /// root that settled, never against whatever the sequencer happens to
    /// hold right now — which is the root a failing sequencer would be most
    /// willing to produce.
    /// Build a holder's proof against the current state.
    ///
    /// `anchored` is the root a Zcash transaction committed. It is passed in
    /// rather than read from the VM so that a proof is always built against a
    /// root that settled, never against whatever the sequencer happens to
    /// hold right now — which is the root a failing sequencer would be most
    /// willing to produce.
    ///
    /// The claim names the note's **owner**, and a custodian pays that
    /// account's bound destination rather than whoever presented the proof.
    /// That is what makes the proof safe to be public: holding a copy of
    /// someone else's proof buys nothing.
    pub fn prove(
        vm: &ObjectVm,
        note: &Note,
        anchored: Hash,
    ) -> Result<ExitProof, ExitError> {
        let c = commitment(note).map_err(|_| ExitError::UnclaimableNote)?;
        let leaves = vm.commitments.leaves();
        let position = leaves
            .iter()
            .position(|l| *l == c)
            .ok_or(ExitError::NotInCommitmentTree)?;

        let live = vm.live_commitments();
        let live_index = live
            .iter()
            .position(|l| *l == c)
            .ok_or(ExitError::NotLive)?;

        Ok(ExitProof {
            note: *note,
            position: position as u64,
            commitment_path: merkle_proof(leaves, position),
            live_path: merkle_proof(&live, live_index),
            sections: vm.sections(),
            anchored,
        })
    }
}

pub mod ingress {
    use super::*;
    use zyn_vm::note_enc::{seal, StaticSecret};

    /// Why a confirmed deposit could not be turned into an object.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum IngressError {
        /// Nobody has said where to pay this account (§register_viewing_key).
        /// The ZEC is in the vault and the credit waits — it is not lost, and
        /// it is not creditable either, because an object nobody can open is
        /// worse than one that has not been made yet.
        NoViewingKey,
        /// The same Zcash transaction has already been credited.
        AlreadyCredited,
        AmountNotPositive,
    }

    /// Build the intent that credits one confirmed deposit.
    ///
    /// `nonce` and `ephemeral` must be fresh and unpredictable. A nonce a
    /// third party can guess makes the commitment openable by trying
    /// plausible amounts against a known account; a reused ephemeral key
    /// links two deposits to the same recipient.
    pub fn credit(
        vm: &ObjectVm,
        asset: Address,
        amount: Fixed,
        account: AccountId,
        txid: Hash,
        nonce: Hash,
        ephemeral: StaticSecret,
    ) -> Result<Op, IngressError> {
        if amount.0 <= 0 {
            return Err(IngressError::AmountNotPositive);
        }
        if vm.deposits.contains(&txid) {
            return Err(IngressError::AlreadyCredited);
        }
        let to = vm
            .viewing
            .get(&account)
            .copied()
            .ok_or(IngressError::NoViewingKey)?;

        // With a bridge fee set, the depositor's note is the deposit less
        // the fee, and the credit says both so the VM can check the split.
        let gross = amount;
        let fee = crate::market::fee_of(gross, vm.market.bridge_bps()).ok_or(IngressError::AmountNotPositive)?;
        let amount = Fixed(gross.0 - fee.0);
        if amount.0 <= 0 {
            return Err(IngressError::AmountNotPositive);
        }

        // What the holder needs to spend it: the asset, the amount, and the
        // nonce. Not the owner — they know who they are, and a ciphertext
        // that names its recipient would say so to anyone who cracked one.
        let mut plain = Vec::with_capacity(80);
        plain.extend_from_slice(&asset);
        plain.extend_from_slice(&amount.0.to_be_bytes());
        plain.extend_from_slice(&nonce);
        let enc = seal(&to, ephemeral, &plain).map_err(|_| IngressError::AmountNotPositive)?;

        if fee.0 > 0 {
            return Ok(Op::DepositWithFee {
                program: program::deposit(),
                asset,
                gross,
                amount,
                owner: account,
                nonce,
                enc,
                external_ref: txid,
            });
        }
        Ok(Op::Deposit {
            program: program::deposit(),
            asset,
            amount,
            owner: account,
            nonce,
            enc,
            external_ref: txid,
        })
    }
}

/// The primary sale's delivery: an item minted to its buyer as a note sealed
/// to the viewing key they registered, so their wallet finds it.
pub mod primary {
    use super::*;
    use zyn_vm::note_enc::{seal, StaticSecret};

    pub fn mint_item(
        vm: &ObjectVm,
        collection: Address,
        serial: u32,
        owner: AccountId,
        nonce: Hash,
        ephemeral: StaticSecret,
    ) -> Result<Op, &'static str> {
        let to = vm.viewing.get(&owner).copied().ok_or("the buyer has not registered where to be paid")?;
        let asset = market::item_asset(&collection, serial);
        let mut plain = Vec::with_capacity(80);
        plain.extend_from_slice(&asset);
        plain.extend_from_slice(&market::ITEM_UNIT.0.to_be_bytes());
        plain.extend_from_slice(&nonce);
        let enc = seal(&to, ephemeral, &plain).map_err(|_| "cannot seal the item")?;
        Ok(Op::MintItem { program: program::mint_item(), collection, serial, owner, nonce, enc })
    }
}

/// What the vault operator has to do, and what anyone can check they did.
pub mod egress {
    use super::*;

    /// One debt the vault owes.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub struct Owed {
        pub index: u32,
        pub asset: Address,
        pub amount: Fixed,
        /// `H(zcash address ‖ salt)`. The operator learns the preimage from
        /// the holder out of band; the chain never carries it, so a reader of
        /// the chain cannot tell where anyone's money went.
        pub destination: Hash,
        /// How long it has been owed, in epochs. The number that matters: an
        /// exit nobody paid is invisible unless someone is counting.
        pub waiting: u64,
    }

    /// Everything unpaid, oldest first.
    ///
    /// Oldest first because it is the order a settler should honour and the
    /// order a reviewer will check, and because "which exit is most overdue"
    /// should not require sorting a list yourself.
    pub fn owed(vm: &ObjectVm) -> Vec<Owed> {
        let mut out: Vec<Owed> = vm
            .exits
            .iter()
            .enumerate()
            .filter(|(_, x)| x.paid_by.is_none())
            .map(|(i, x)| Owed {
                index: i as u32,
                asset: x.asset,
                amount: x.amount,
                destination: x.destination,
                waiting: vm.epoch.saturating_sub(x.since_epoch),
            })
            .collect();
        out.sort_by_key(|o| core::cmp::Reverse(o.waiting));
        out
    }

    /// The intent that records a payment.
    ///
    /// Deliberately separate from making the payment. The vault spends on
    /// Zcash first and says so afterwards: an operator that recorded a
    /// settlement it had not made would be claiming a transaction anyone can
    /// look up and fail to find.
    pub fn settle(index: u32, txid: Hash) -> Op {
        Op::SettleExit {
            program: program::settle_exit(),
            index,
            txid,
        }
    }

    /// What the vault still owes, per asset. The number to compare against
    /// what it actually holds on Zcash before believing it is solvent.
    pub fn outstanding(vm: &ObjectVm, asset: &Address) -> Fixed {
        Fixed(
            vm.exits
                .iter()
                .filter(|x| x.paid_by.is_none() && &x.asset == asset)
                .fold(0i128, |a, x| a.saturating_add(x.amount.0)),
        )
    }
}
