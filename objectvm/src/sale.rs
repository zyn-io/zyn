//! A collection's primary sale: lottery tickets, a posted draw, trustless
//! claims and an open mint (DECISIONS §144).
//!
//! The shape is the one Cave's published terms describe — a ticket per entry,
//! a draw after an announced close that decides how many items each ticket
//! wins and which, a claim price per item, and an open mint for whatever the
//! draw did not hand out — paid in ZEC.zy on Zyn instead of Zcash deposits.
//!
//! * **Tickets** (`BuyTickets`): the price times the count leaves the buyer's
//!   notes; the treasury's share is owed to the collection's creator as a
//!   claim and the rest joins the collection's redemption floor, here, by
//!   consensus. Each ticket is recorded under an id only its buyer can derive
//!   (`ticket_id(buyer, nonce, k)`), with its place in the sale's order.
//! * **Close and draw** are the operator's: `CloseSale` records the Zcash
//!   block the sale closed at and stops tickets; the draw is run off chain by
//!   `cave-randomness` over the tickets in their order and the published
//!   reveal, and `PostDraw` commits its result as a Merkle root.
//! * **Claims** (`ClaimItems`) are a winner's, and the operator is not in
//!   them: the chain checks that the ticket is the claimant's, that the
//!   serials are what the posted root says it drew, takes the claim price and
//!   mints the items straight to them. Nobody can be given an item the draw
//!   did not give them, or be refused one it did.
//! * **Open mint** (`PostOpenMint`, `MintOpen`): the operator ends claims and
//!   commits the order the rest are sold in; anyone then mints the next one
//!   at the open price, proved against that root.
//!
//! * **Open-mint terms** (`SetOpenTerms`, DECISIONS §187): accounts on the
//!   sale's list (the gate's) mint at a lower price, and no account mints
//!   more than a cap. Set before the first mint; the list may still grow
//!   during the open mint.
//!
//! * **Burn tickets** (`SetVoucherCap`, `PostVoucher`, `ClaimVoucherItems`,
//!   DECISIONS §145): the operator records a burn of a promotional token on
//!   another chain, once, and it becomes tickets in the same list, under ids
//!   derived from a commitment the burner put in the burn's memo (their Zyn
//!   account and a secret). They pay nothing in, so a sale caps how many there
//!   may be; their winners claim by revealing the secret from that account and
//!   pay the claim price like anyone.
//!
//! * **Sale credit** (`FundSaleCredit`, `PostCredit`, `RedeemCredit`,
//!   DECISIONS §182): the team pays ZEC.zy into a sale as an ordinary sale
//!   payment — the floor's and the treasury's shares at once — and the
//!   amount becomes the sale's credit budget. The operator turns one burn of
//!   a token on another chain, once, into credit worth what was burned,
//!   under the burner's commitment, out of that budget. The burner takes it
//!   up from their account by revealing the commitment's secret, and from
//!   then on their tickets, claims and open mints are paid from it before
//!   any note. Credit is spent only in its sale and is never paid out.
//!
//! The sale holds no value of its own: every payment is in the floor or a
//! claim the moment it is made — credit included, which is paid in before it
//! exists — so conservation needs nothing new.

use zyn_vm::commit::{merkle_root, verify_proof, Encoder, Hash, ProofStep};
use zyn_vm::derive::Address;
use zyn_vm::object::commitment;
use zyn_vm::read::Decoder;
use zyn_vm::spec::AccountId;
use zyn_vm::Fixed;

use crate::market::{item_asset, CollectionTerms, ItemInfo, BPS, ITEM_UNIT};
use crate::{ObjectVm, Output, Receipt};

/// Tickets one intent may buy.
pub const MAX_TICKETS_PER_BUY: u32 = 20;
/// Items one ticket can win (the published odds: one, two or three).
pub const MAX_ITEMS_PER_TICKET: usize = 3;
/// Depth of a proof against a posted root: 2^32 leaves is more than any sale.
pub const MAX_PROOF: usize = 32;
/// The most of a sale's takings the treasury may keep.
pub const MAX_TREASURY_BPS: u32 = 2_000;

fn id(domain: &[u8], parts: &[&[u8]]) -> Hash {
    let mut e = Encoder::new();
    e.bytes(domain);
    for p in parts {
        e.bytes(p);
    }
    e.leaf()
}

/// One ticket's id: from the buyer, the nonce they bought with, and its place
/// in that purchase. Only the buyer can derive it, so a public list of
/// tickets says nothing about who holds them.
pub fn ticket_id(buyer: &AccountId, nonce: &Hash, k: u32) -> Hash {
    id(b"objectvm.ticket.v1", &[buyer, nonce, &k.to_be_bytes()])
}

/// What a burner puts in the burn's memo: a commitment to their Zyn account
/// and a secret only their wallet holds. A claim reveals the secret and is
/// checked against the claimant's own account, so a revealed secret lets
/// nobody else claim the burn's other tickets.
pub fn voucher_commitment(account: &AccountId, secret: &Hash) -> Hash {
    id(b"objectvm.voucher.v1", &[account, secret])
}

/// A burn ticket's id: from the burner's commitment and its place in the burn.
pub fn voucher_ticket_id(commitment: &Hash, k: u32) -> Hash {
    id(b"objectvm.voucher-ticket.v1", &[commitment, &k.to_be_bytes()])
}

/// Where the treasury's share of one payment is claimable: from the sale and
/// the payment, both public, so the creator can find it without asking.
pub fn treasury_source(collection: &Address, payment: &Hash) -> Hash {
    id(b"objectvm.sale-treasury.v1", &[collection, payment])
}

/// One leaf of a posted draw: a ticket and the serials it won.
pub fn draw_leaf(ticket: &Hash, serials: &[u32]) -> Hash {
    let mut e = Encoder::new();
    e.bytes(b"objectvm.draw-leaf.v1").bytes(ticket).u32(serials.len() as u32);
    for s in serials {
        e.u32(*s);
    }
    e.leaf()
}

/// One leaf of the open-mint order: its position and the serial sold there.
pub fn open_leaf(position: u32, serial: u32) -> Hash {
    let mut e = Encoder::new();
    e.bytes(b"objectvm.open-leaf.v1").u32(position).u32(serial);
    e.leaf()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    /// Taking tickets.
    Open,
    /// Closed at a Zcash block; waiting for the draw.
    Closed,
    /// The draw is posted; winners claim.
    Drawn,
    /// Claims are over; the rest sell in the posted order.
    OpenMint,
}

impl Phase {
    fn code(self) -> u8 {
        match self {
            Phase::Open => 0,
            Phase::Closed => 1,
            Phase::Drawn => 2,
            Phase::OpenMint => 3,
        }
    }

    fn from(c: u8) -> Option<Phase> {
        Some(match c {
            0 => Phase::Open,
            1 => Phase::Closed,
            2 => Phase::Drawn,
            3 => Phase::OpenMint,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Sale {
    pub collection: Address,
    pub ticket_price: Fixed,
    pub claim_price: Fixed,
    pub open_price: Fixed,
    pub treasury_bps: u32,
    pub phase: Phase,
    /// Tickets sold: also the next ticket's place in the order.
    pub tickets: u32,
    pub close_zcash_height: u32,
    pub close_zcash_hash: Hash,
    pub draw_root: Hash,
    pub open_root: Hash,
    pub open_count: u32,
    pub open_next: u32,
    /// The most burn tickets this sale takes (§145); none until it is set.
    pub voucher_cap: u32,
    /// Burn tickets posted so far.
    pub voucher_tickets: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Ticket {
    pub collection: Address,
    /// Its place in the sale's order — the order the draw runs over.
    pub index: u32,
    pub epoch: u64,
    pub claimed: bool,
}

fn sale_leaf(s: &Sale) -> Hash {
    let mut e = Encoder::new();
    e.bytes(b"objectvm.sale-leaf.v1")
        .bytes(&s.collection)
        .fixed(s.ticket_price)
        .fixed(s.claim_price)
        .fixed(s.open_price)
        .u32(s.treasury_bps)
        .u8(s.phase.code())
        .u32(s.tickets)
        .u32(s.close_zcash_height)
        .bytes(&s.close_zcash_hash)
        .bytes(&s.draw_root)
        .bytes(&s.open_root)
        .u32(s.open_count)
        .u32(s.open_next)
        .u32(s.voucher_cap)
        .u32(s.voucher_tickets);
    e.leaf()
}

fn ticket_leaf(tid: &Hash, t: &Ticket) -> Hash {
    let mut e = Encoder::new();
    e.bytes(b"objectvm.ticket-leaf.v1")
        .bytes(tid)
        .bytes(&t.collection)
        .u32(t.index)
        .u64(t.epoch)
        .bool(t.claimed);
    e.leaf()
}

/// The sales section of the market's leaf: every sale in the clear, and the
/// tickets as a root of leaves keyed by ids nobody else can derive.
pub(crate) fn section(
    e: &mut Encoder,
    sales: &std::collections::BTreeMap<Address, Sale>,
    tickets: &std::collections::BTreeMap<Hash, Ticket>,
    burns: &std::collections::BTreeMap<Hash, Address>,
) {
    let s: Vec<Hash> = sales.values().map(sale_leaf).collect();
    let t: Vec<Hash> = tickets.iter().map(|(i, t)| ticket_leaf(i, t)).collect();
    let b: Vec<Hash> = burns.iter().map(|(burn, c)| id(b"objectvm.burn-leaf.v1", &[burn, c])).collect();
    e.bytes(b"objectvm.sales.v1")
        .u32(sales.len() as u32)
        .bytes(&merkle_root(&s))
        .u32(tickets.len() as u32)
        .bytes(&merkle_root(&t))
        .u32(burns.len() as u32)
        .bytes(&merkle_root(&b));
}

pub(crate) fn encode(
    e: &mut Encoder,
    sales: &std::collections::BTreeMap<Address, Sale>,
    tickets: &std::collections::BTreeMap<Hash, Ticket>,
    burns: &std::collections::BTreeMap<Hash, Address>,
) {
    e.u32(sales.len() as u32);
    for s in sales.values() {
        e.bytes(&s.collection)
            .fixed(s.ticket_price)
            .fixed(s.claim_price)
            .fixed(s.open_price)
            .u32(s.treasury_bps)
            .u8(s.phase.code())
            .u32(s.tickets)
            .u32(s.close_zcash_height)
            .bytes(&s.close_zcash_hash)
            .bytes(&s.draw_root)
            .bytes(&s.open_root)
            .u32(s.open_count)
            .u32(s.open_next)
            .u32(s.voucher_cap)
            .u32(s.voucher_tickets);
    }
    e.u32(tickets.len() as u32);
    for (i, t) in tickets {
        e.bytes(i).bytes(&t.collection).u32(t.index).u64(t.epoch).bool(t.claimed);
    }
    e.u32(burns.len() as u32);
    for (burn, c) in burns {
        e.bytes(burn).bytes(c);
    }
}

/// Sales, tickets and the burns already posted, as a snapshot holds them.
pub(crate) type SaleState = (
    std::collections::BTreeMap<Address, Sale>,
    std::collections::BTreeMap<Hash, Ticket>,
    std::collections::BTreeMap<Hash, Address>,
);

/// `v6` snapshots carry burn tickets (§145); a `v5` one reads as a sale
/// with no cap and no burns.
pub(crate) fn decode(d: &mut Decoder, v6: bool) -> Option<SaleState> {
    let mut sales = std::collections::BTreeMap::new();
    for _ in 0..d.u32().ok()? {
        let collection = d.hash().ok()?;
        sales.insert(
            collection,
            Sale {
                collection,
                ticket_price: d.fixed().ok()?,
                claim_price: d.fixed().ok()?,
                open_price: d.fixed().ok()?,
                treasury_bps: d.u32().ok()?,
                phase: Phase::from(d.u8().ok()?)?,
                tickets: d.u32().ok()?,
                close_zcash_height: d.u32().ok()?,
                close_zcash_hash: d.hash().ok()?,
                draw_root: d.hash().ok()?,
                open_root: d.hash().ok()?,
                open_count: d.u32().ok()?,
                open_next: d.u32().ok()?,
                voucher_cap: if v6 { d.u32().ok()? } else { 0 },
                voucher_tickets: if v6 { d.u32().ok()? } else { 0 },
            },
        );
    }
    let mut tickets = std::collections::BTreeMap::new();
    for _ in 0..d.u32().ok()? {
        let tid = d.hash().ok()?;
        tickets.insert(
            tid,
            Ticket {
                collection: d.hash().ok()?,
                index: d.u32().ok()?,
                epoch: d.u64().ok()?,
                claimed: d.u8().ok()? == 1,
            },
        );
    }
    let mut burns = std::collections::BTreeMap::new();
    if v6 {
        for _ in 0..d.u32().ok()? {
            burns.insert(d.hash().ok()?, d.hash().ok()?);
        }
    }
    Some((sales, tickets, burns))
}

// ---------------------------------------------------------------------------
// Sale credit (§182). Beside the sale, not in it: a chain with no credit
// hashes and saves exactly as before.

/// Where a burn's credit waits: by sale and the burner's commitment
/// (`voucher_commitment`, the same one a burn ticket's memo carries).
pub fn credit_pending_key(collection: &Address, commitment: &Hash) -> Hash {
    id(b"objectvm.sale-credit-pending.v1", &[collection, commitment])
}

/// Where an account's credit in a sale is held once taken up: by a key that
/// names the account only to whoever already knows it.
pub fn credit_key(collection: &Address, account: &AccountId) -> Hash {
    id(b"objectvm.sale-credit.v1", &[collection, account])
}

/// The id a team funding is paid under, so its treasury share has a home.
pub fn credit_funding(funder: &AccountId, nonce: &Hash) -> Hash {
    id(b"objectvm.sale-credit-fund.v1", &[funder, nonce])
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Credits {
    /// What the team has paid in for burns and not yet handed out, by sale.
    pub budgets: std::collections::BTreeMap<Address, Fixed>,
    /// Credit posted for a burn and not yet taken up, by `credit_pending_key`.
    pub pending: std::collections::BTreeMap<Hash, Fixed>,
    /// Credit taken up, by `credit_key`.
    pub held: std::collections::BTreeMap<Hash, Fixed>,
}

impl Credits {
    pub fn is_empty(&self) -> bool {
        self.budgets.is_empty() && self.pending.is_empty() && self.held.is_empty()
    }
}

fn fixed_leaves(domain: &[u8], m: &std::collections::BTreeMap<Hash, Fixed>) -> (u32, Hash) {
    let leaves: Vec<Hash> = m
        .iter()
        .map(|(k, v)| {
            let mut e = Encoder::new();
            e.bytes(domain).bytes(k).fixed(*v);
            e.leaf()
        })
        .collect();
    (leaves.len() as u32, merkle_root(&leaves))
}

/// Credit's part of the market's leaf, joined only once there is any.
pub(crate) fn credits_section(e: &mut Encoder, c: &Credits) {
    let (nb, b) = fixed_leaves(b"objectvm.sale-credit-budget-leaf.v1", &c.budgets);
    let (np, p) = fixed_leaves(b"objectvm.sale-credit-pending-leaf.v1", &c.pending);
    let (nh, h) = fixed_leaves(b"objectvm.sale-credit-held-leaf.v1", &c.held);
    e.bytes(b"objectvm.sale-credits.v1").u32(nb).bytes(&b).u32(np).bytes(&p).u32(nh).bytes(&h);
}

pub(crate) fn encode_credits(e: &mut Encoder, c: &Credits) {
    for m in [&c.budgets, &c.pending, &c.held] {
        e.u32(m.len() as u32);
        for (k, v) in m {
            e.bytes(k).fixed(*v);
        }
    }
}

pub(crate) fn decode_credits(d: &mut Decoder) -> Option<Credits> {
    let mut read = || -> Option<std::collections::BTreeMap<Hash, Fixed>> {
        let mut m = std::collections::BTreeMap::new();
        for _ in 0..d.u32().ok()? {
            m.insert(d.hash().ok()?, d.fixed().ok()?);
        }
        Some(m)
    };
    Some(Credits { budgets: read()?, pending: read()?, held: read()? })
}

impl ObjectVm {
    fn sale(&self, collection: &Address) -> Result<(Sale, CollectionTerms), &'static str> {
        let s = *self.market.sales.get(collection).ok_or("that collection has no sale")?;
        let t = *self.market.collections.get(collection).ok_or("register the collection first")?;
        Ok((s, t))
    }

    /// Take `amount` for the sale from `who`: from their credit in it first
    /// (§182), which was paid into the floor and the treasury when the team
    /// funded it, and the rest from their notes.
    #[allow(clippy::too_many_arguments)]
    fn take_sale_payment(
        &mut self,
        who: &AccountId,
        collection: &Address,
        payment: &Hash,
        amount: Fixed,
        inputs: &[(Hash, Hash)],
        change: &[Output],
    ) -> Result<(), &'static str> {
        let key = credit_key(collection, who);
        let credit = self.market.credits.held.get(&key).map_or(0, |c| c.0);
        if credit <= 0 {
            return self.pay_into_sale(who, collection, payment, amount, inputs, change);
        }
        let used = credit.min(amount.0);
        if used < amount.0 {
            self.pay_into_sale(who, collection, payment, Fixed(amount.0 - used), inputs, change)?;
        } else if !inputs.is_empty() || !change.is_empty() {
            return Err("credit covers this payment; spend no notes");
        }
        if used == credit {
            self.market.credits.held.remove(&key);
        } else {
            self.market.credits.held.insert(key, Fixed(credit - used));
        }
        Ok(())
    }

    /// Take `amount` of the collection's quote asset from `who`'s notes: the
    /// treasury's share owed to the creator, the rest into the floor.
    #[allow(clippy::too_many_arguments)]
    fn pay_into_sale(
        &mut self,
        who: &AccountId,
        collection: &Address,
        payment: &Hash,
        amount: Fixed,
        inputs: &[(Hash, Hash)],
        change: &[Output],
    ) -> Result<(), &'static str> {
        let (s, t) = self.sale(collection)?;
        let f = self.plan_funding(who, inputs, change, &[(t.quote, amount)])?;
        let treasury = zyn_vm::fixed::mul_div(amount.0, s.treasury_bps as i128, BPS).ok_or("payment overflow")?;
        self.apply_funding(who, inputs, f);
        let t = *self.market.collections.get(collection).expect("checked above");
        self.market.collections.insert(*collection, CollectionTerms { floor: Fixed(t.floor.0 + amount.0 - treasury), ..t });
        // To the collection's DAO treasury once it has one (§172).
        self.pay_creator_share(collection, &treasury_source(collection, payment), t.creator, t.quote, Fixed(treasury));
        Ok(())
    }

    /// Everything an item output must be before it is minted: the serial's
    /// asset, one unit, the claimant's, findable, new, and within the cap.
    fn check_item(&self, collection: &Address, t: &CollectionTerms, serial: u32, owner: &AccountId, out: &Output) -> Result<Hash, &'static str> {
        if serial >= t.cap {
            return Err("that serial is beyond the collection's cap");
        }
        let asset = item_asset(collection, serial);
        if self.market.items.contains_key(&asset) || self.issued.contains_key(&asset) {
            return Err("that item was already minted");
        }
        if out.note.asset != asset || out.note.amount != ITEM_UNIT || &out.note.owner != owner {
            return Err("an item output must be that serial, one unit, to the claimant");
        }
        if out.enc.ciphertext.is_empty() {
            return Err("an item needs an encrypted note, or its owner cannot find it");
        }
        let c = commitment(&out.note).map_err(|_| "an item needs a nonce")?;
        if self.live.contains_key(&c) {
            return Err("that object already exists");
        }
        Ok(c)
    }

    fn mint_checked(&mut self, collection: &Address, serial: u32, out: &Output, c: Hash) -> Result<Hash, &'static str> {
        let asset = item_asset(collection, serial);
        self.issue(asset, ITEM_UNIT)?;
        self.commitments.append(c);
        self.live.insert(c, out.note);
        self.accounts.entry(out.note.owner).or_insert(0);
        self.market.items.insert(asset, ItemInfo { collection: *collection, serial });
        let t = *self.market.collections.get(collection).expect("minting for a registered collection");
        self.market
            .collections
            .insert(*collection, CollectionTerms { minted: t.minted + 1, outstanding: t.outstanding + 1, ..t });
        Ok(c)
    }

    pub(crate) fn apply_set_sale(
        &mut self,
        collection: &Address,
        ticket_price: Fixed,
        claim_price: Fixed,
        open_price: Fixed,
        treasury_bps: u32,
    ) -> Result<Receipt, &'static str> {
        if !self.market.collections.contains_key(collection) {
            return Err("register the collection first");
        }
        if ticket_price.0 <= 0 || claim_price.0 <= 0 || open_price.0 <= 0 {
            return Err("a sale's prices are above nothing");
        }
        if treasury_bps > MAX_TREASURY_BPS {
            return Err("the treasury's share is above the ceiling");
        }
        let mut voucher_cap = 0;
        if let Some(s) = self.market.sales.get(collection) {
            // Before its first ticket a sale may be re-priced. After a close
            // with none, nothing can be drawn and nobody holds anything, so it
            // may be set again rather than stay closed for good.
            let untouched = s.tickets == 0 && (s.phase == Phase::Open || s.phase == Phase::Closed);
            if !untouched {
                return Err("the sale has started; its prices are fixed");
            }
            voucher_cap = s.voucher_cap;
        }
        self.market.sales.insert(
            *collection,
            Sale {
                collection: *collection,
                ticket_price,
                claim_price,
                open_price,
                treasury_bps,
                phase: Phase::Open,
                tickets: 0,
                close_zcash_height: 0,
                close_zcash_hash: [0; 32],
                draw_root: [0; 32],
                open_root: [0; 32],
                open_count: 0,
                open_next: 0,
                voucher_cap,
                voucher_tickets: 0,
            },
        );
        Ok(Receipt::SaleOpened { collection: *collection })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_buy_tickets(
        &mut self,
        buyer: &AccountId,
        collection: &Address,
        count: u32,
        nonce: &Hash,
        inputs: &[(Hash, Hash)],
        change: &[Output],
    ) -> Result<Receipt, &'static str> {
        let (s, _) = self.sale(collection)?;
        if s.phase != Phase::Open {
            return Err("the sale is closed");
        }
        if count == 0 || count > MAX_TICKETS_PER_BUY {
            return Err("buy between 1 and 20 tickets at a time");
        }
        let first = ticket_id(buyer, nonce, 0);
        if self.market.tickets.contains_key(&first) {
            return Err("that nonce was already used");
        }
        // A gated sale (§168) sells only to listed accounts, each at the
        // base price for its first tickets and more for each after.
        let key = allow_key(collection, buyer);
        let total = match self.market.sale_gates.get(collection) {
            None => s.ticket_price.0.checked_mul(count as i128).ok_or("payment overflow")?,
            Some(g) => {
                let bought = *g.allowed.get(&key).ok_or("only accounts on the sale's list can buy tickets")?;
                gated_total(s.ticket_price, g.base, bought, count).ok_or("payment overflow")?
            }
        };
        self.take_sale_payment(buyer, collection, &first, Fixed(total), inputs, change)?;
        if let Some(n) = self.market.sale_gates.get_mut(collection).and_then(|g| g.allowed.get_mut(&key)) {
            *n += count;
        }
        let epoch = self.epoch;
        for k in 0..count {
            self.market
                .tickets
                .insert(ticket_id(buyer, nonce, k), Ticket { collection: *collection, index: s.tickets + k, epoch, claimed: false });
        }
        self.market.sales.get_mut(collection).expect("checked above").tickets = s.tickets + count;
        Ok(Receipt::TicketsBought { first, count })
    }

    /// How many burn tickets the sale takes (§145). Set while it is open, and
    /// never below what has already been posted.
    pub(crate) fn apply_set_voucher_cap(&mut self, collection: &Address, cap: u32) -> Result<Receipt, &'static str> {
        let (s, _) = self.sale(collection)?;
        if s.phase != Phase::Open {
            return Err("the sale is closed");
        }
        if cap < s.voucher_tickets {
            return Err("more burn tickets than that are already posted");
        }
        self.market.sales.get_mut(collection).expect("checked above").voucher_cap = cap;
        Ok(Receipt::VoucherCapSet { collection: *collection, cap })
    }

    /// One burn, recorded once, becomes `count` tickets in the sale's order
    /// under ids from the burner's commitment (§145).
    pub(crate) fn apply_post_voucher(&mut self, collection: &Address, burn: &Hash, commitment: &Hash, count: u32) -> Result<Receipt, &'static str> {
        let (s, _) = self.sale(collection)?;
        if s.phase != Phase::Open {
            return Err("the sale is closed");
        }
        if count == 0 || count > MAX_TICKETS_PER_BUY {
            return Err("a burn is worth between 1 and 20 tickets");
        }
        if burn == &[0; 32] || commitment == &[0; 32] {
            return Err("a burn names itself and a commitment");
        }
        if self.market.burns.contains_key(burn) {
            return Err("that burn is already posted");
        }
        if s.voucher_tickets.checked_add(count).is_none_or(|n| n > s.voucher_cap) {
            return Err("the sale's burn tickets are all taken");
        }
        let first = voucher_ticket_id(commitment, 0);
        if self.market.tickets.contains_key(&first) {
            return Err("that commitment was already used; burn with a fresh one");
        }
        let epoch = self.epoch;
        for k in 0..count {
            self.market
                .tickets
                .insert(voucher_ticket_id(commitment, k), Ticket { collection: *collection, index: s.tickets + k, epoch, claimed: false });
        }
        self.market.burns.insert(*burn, *collection);
        let sale = self.market.sales.get_mut(collection).expect("checked above");
        sale.tickets = s.tickets + count;
        sale.voucher_tickets = s.voucher_tickets + count;
        Ok(Receipt::VoucherPosted { first, count })
    }

    /// The team pays into a sale for burns to be credited against (§182): an
    /// ordinary payment — floor and treasury at once — that becomes budget.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_fund_sale_credit(
        &mut self,
        funder: &AccountId,
        collection: &Address,
        amount: Fixed,
        nonce: &Hash,
        inputs: &[(Hash, Hash)],
        change: &[Output],
    ) -> Result<Receipt, &'static str> {
        let (s, _) = self.sale(collection)?;
        if s.phase == Phase::OpenMint && s.open_next >= s.open_count {
            return Err("the sale is over");
        }
        if amount.0 <= 0 {
            return Err("fund a sale with something");
        }
        let payment = credit_funding(funder, nonce);
        if self.market.claims.contains_key(&crate::market::claim_id(&treasury_source(collection, &payment), 0)) {
            return Err("that nonce was already used");
        }
        // Notes only: credit never funds credit.
        self.pay_into_sale(funder, collection, &payment, amount, inputs, change)?;
        let budget = self.market.credits.budgets.get(collection).map_or(0, |b| b.0);
        let budget = budget.checked_add(amount.0).ok_or("budget overflow")?;
        self.market.credits.budgets.insert(*collection, Fixed(budget));
        Ok(Receipt::SaleCreditFunded { collection: *collection, budget: Fixed(budget) })
    }

    /// One burn on another chain, recorded once, becomes credit worth what it
    /// burned, out of the sale's budget, under the burner's commitment (§182).
    pub(crate) fn apply_post_credit(&mut self, collection: &Address, burn: &Hash, commitment: &Hash, amount: Fixed) -> Result<Receipt, &'static str> {
        let (s, _) = self.sale(collection)?;
        if s.phase == Phase::OpenMint && s.open_next >= s.open_count {
            return Err("the sale is over");
        }
        if amount.0 <= 0 {
            return Err("a burn's credit is worth something");
        }
        if burn == &[0; 32] || commitment == &[0; 32] {
            return Err("a burn names itself and a commitment");
        }
        if self.market.burns.contains_key(burn) {
            return Err("that burn is already posted");
        }
        let budget = self.market.credits.budgets.get(collection).map_or(0, |b| b.0);
        if amount.0 > budget {
            return Err("the sale's credit budget does not cover that burn");
        }
        let key = credit_pending_key(collection, commitment);
        let pending = self.market.credits.pending.get(&key).map_or(0, |p| p.0).checked_add(amount.0).ok_or("credit overflow")?;
        self.market.burns.insert(*burn, *collection);
        if budget == amount.0 {
            self.market.credits.budgets.remove(collection);
        } else {
            self.market.credits.budgets.insert(*collection, Fixed(budget - amount.0));
        }
        self.market.credits.pending.insert(key, Fixed(pending));
        Ok(Receipt::CreditPosted { key, amount })
    }

    /// Take up a burn's credit (§182): revealing the secret behind its
    /// commitment, from the account the commitment names, moves it there.
    pub(crate) fn apply_redeem_credit(&mut self, buyer: &AccountId, collection: &Address, secret: &Hash) -> Result<Receipt, &'static str> {
        self.sale(collection)?;
        let key = credit_pending_key(collection, &voucher_commitment(buyer, secret));
        let amount = *self.market.credits.pending.get(&key).ok_or("no credit waits for this account")?;
        let held = credit_key(collection, buyer);
        let total = self.market.credits.held.get(&held).map_or(0, |h| h.0).checked_add(amount.0).ok_or("credit overflow")?;
        self.market.credits.pending.remove(&key);
        self.market.credits.held.insert(held, Fixed(total));
        Ok(Receipt::CreditRedeemed { amount })
    }

    pub(crate) fn apply_close_sale(&mut self, collection: &Address, zcash_height: u32, zcash_hash: &Hash) -> Result<Receipt, &'static str> {
        let (s, _) = self.sale(collection)?;
        if s.phase != Phase::Open {
            return Err("the sale is not open");
        }
        if zcash_height == 0 || zcash_hash == &[0; 32] {
            return Err("a close names the Zcash block it happened at");
        }
        let s = self.market.sales.get_mut(collection).expect("checked above");
        s.phase = Phase::Closed;
        s.close_zcash_height = zcash_height;
        s.close_zcash_hash = *zcash_hash;
        Ok(Receipt::SaleClosed { collection: *collection, tickets: s.tickets })
    }

    pub(crate) fn apply_post_draw(&mut self, collection: &Address, root: &Hash) -> Result<Receipt, &'static str> {
        let (s, _) = self.sale(collection)?;
        if s.phase != Phase::Closed {
            return Err("the draw follows the close, once");
        }
        if root == &[0; 32] {
            return Err("a draw posts a root");
        }
        let s = self.market.sales.get_mut(collection).expect("checked above");
        s.phase = Phase::Drawn;
        s.draw_root = *root;
        Ok(Receipt::DrawPosted { collection: *collection })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_claim_items(
        &mut self,
        buyer: &AccountId,
        collection: &Address,
        nonce: &Hash,
        k: u32,
        serials: &[u32],
        proof: &[ProofStep],
        items: &[Output],
        inputs: &[(Hash, Hash)],
        change: &[Output],
    ) -> Result<Receipt, &'static str> {
        self.claim_ticket(buyer, collection, ticket_id(buyer, nonce, k), serials, proof, items, inputs, change)
    }

    /// Claim what burn ticket `(secret, k)` drew (§145): revealing the secret
    /// behind the burn's commitment is what shows the tickets are the
    /// claimant's; the rest is an ordinary claim.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_claim_voucher_items(
        &mut self,
        buyer: &AccountId,
        collection: &Address,
        secret: &Hash,
        k: u32,
        serials: &[u32],
        proof: &[ProofStep],
        items: &[Output],
        inputs: &[(Hash, Hash)],
        change: &[Output],
    ) -> Result<Receipt, &'static str> {
        // Burn tickets are the list's too in a gated sale (§168).
        if !self.sale_allows(collection, buyer) {
            return Err("only accounts on the sale's list can claim burn tickets");
        }
        self.claim_ticket(buyer, collection, voucher_ticket_id(&voucher_commitment(buyer, secret), k), serials, proof, items, inputs, change)
    }

    #[allow(clippy::too_many_arguments)]
    fn claim_ticket(
        &mut self,
        buyer: &AccountId,
        collection: &Address,
        tid: Hash,
        serials: &[u32],
        proof: &[ProofStep],
        items: &[Output],
        inputs: &[(Hash, Hash)],
        change: &[Output],
    ) -> Result<Receipt, &'static str> {
        let (s, t) = self.sale(collection)?;
        if s.phase != Phase::Drawn {
            return Err("claims are open between the draw and the open mint");
        }
        let ticket = *self.market.tickets.get(&tid).ok_or("no such ticket is yours")?;
        if &ticket.collection != collection {
            return Err("that ticket is for another sale");
        }
        if ticket.claimed {
            return Err("that ticket was already claimed");
        }
        if serials.is_empty() || serials.len() > MAX_ITEMS_PER_TICKET || items.len() != serials.len() {
            return Err("a claim names each item it won, once");
        }
        if proof.len() > MAX_PROOF || !verify_proof(draw_leaf(&tid, serials), proof, s.draw_root) {
            return Err("that is not what this ticket drew");
        }
        let mut made = Vec::with_capacity(serials.len());
        for (i, (serial, out)) in serials.iter().zip(items).enumerate() {
            if serials[..i].contains(serial) {
                return Err("a claim names each item it won, once");
            }
            made.push(self.check_item(collection, &t, *serial, buyer, out)?);
        }
        if made.iter().enumerate().any(|(i, c)| made[..i].contains(c)) {
            return Err("that object already exists");
        }
        let total = s.claim_price.0.checked_mul(serials.len() as i128).ok_or("payment overflow")?;
        self.take_sale_payment(buyer, collection, &tid, Fixed(total), inputs, change)?;
        for ((serial, out), c) in serials.iter().zip(items).zip(made.iter()) {
            self.mint_checked(collection, *serial, out, *c)?;
        }
        self.market.tickets.get_mut(&tid).expect("checked above").claimed = true;
        Ok(Receipt::ItemsClaimed { created: made })
    }

    pub(crate) fn apply_post_open_mint(&mut self, collection: &Address, root: &Hash, count: u32) -> Result<Receipt, &'static str> {
        let (s, _) = self.sale(collection)?;
        if s.phase != Phase::Drawn && s.phase != Phase::Closed {
            return Err("the open mint follows the close");
        }
        if count > 0 && root == &[0; 32] {
            return Err("an open mint posts a root");
        }
        let s = self.market.sales.get_mut(collection).expect("checked above");
        s.phase = Phase::OpenMint;
        s.open_root = *root;
        s.open_count = count;
        s.open_next = 0;
        Ok(Receipt::OpenMintPosted { collection: *collection, count })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_mint_open(
        &mut self,
        buyer: &AccountId,
        collection: &Address,
        serial: u32,
        proof: &[ProofStep],
        item: &Output,
        inputs: &[(Hash, Hash)],
        change: &[Output],
        nonce: &Hash,
    ) -> Result<Receipt, &'static str> {
        let (s, t) = self.sale(collection)?;
        if s.phase != Phase::OpenMint {
            return Err("the open mint has not started");
        }
        if s.open_next >= s.open_count {
            return Err("the open mint is sold out");
        }
        if proof.len() > MAX_PROOF || !verify_proof(open_leaf(s.open_next, serial), proof, s.open_root) {
            return Err("that is not the next item in the open mint");
        }
        let c = self.check_item(collection, &t, serial, buyer, item)?;
        let payment = id(b"objectvm.open-payment.v1", &[buyer, nonce]);
        if self.market.claims.contains_key(&crate::market::claim_id(&treasury_source(collection, &payment), 0)) {
            return Err("that nonce was already used");
        }
        // Open-mint terms (§187): the cap, and the listed price for accounts
        // on the sale's list.
        let key = allow_key(collection, buyer);
        let price = match self.market.open_terms.get(collection) {
            None => s.open_price,
            Some(o) => {
                if o.minted.get(&key).copied().unwrap_or(0) >= o.cap {
                    return Err("this account has minted as many as one account may");
                }
                if self.market.sale_gates.get(collection).is_some_and(|g| g.allowed.contains_key(&key)) { o.listed_price } else { s.open_price }
            }
        };
        self.take_sale_payment(buyer, collection, &payment, price, inputs, change)?;
        self.mint_checked(collection, serial, item, c)?;
        self.market.sales.get_mut(collection).expect("checked above").open_next = s.open_next + 1;
        if let Some(o) = self.market.open_terms.get_mut(collection) {
            *o.minted.entry(key).or_insert(0) += 1;
        }
        Ok(Receipt::ItemsClaimed { created: vec![c] })
    }
}

// ---------------------------------------------------------------------------
// The whitelist gate (§168): only listed accounts buy tickets or claim burn
// tickets, and each listed account pays the base price for its first `base`
// tickets and one more base price for every ticket after — the 11th at twice
// the price, the 12th at three times. Beside the sale, not in it, so a sale
// without a gate hashes and saves exactly as before.

/// Accounts one `AllowBuyers` adds at most.
pub const MAX_ALLOW_PER_OP: usize = 256;

/// A gated sale's list: blinded keys of the accounts on it, and how many
/// tickets each has bought.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Gate {
    /// Tickets each account buys at the base price before it rises.
    pub base: u32,
    pub allowed: std::collections::BTreeMap<Hash, u32>,
}

/// An account's place on a sale's list, by a key that names it only to whoever
/// already knows the account.
pub fn allow_key(collection: &Address, account: &AccountId) -> Hash {
    id(b"objectvm.sale-allow.v1", &[collection, account])
}

/// What the account's `n`th ticket (from 1) costs: the base price up to
/// `base`, then one more base price for each ticket past it.
pub fn gated_price(price: Fixed, base: u32, n: u32) -> Option<i128> {
    let step = if n <= base { 1 } else { (n - base + 1) as i128 };
    price.0.checked_mul(step)
}

/// What tickets `bought+1 ..= bought+count` cost together.
pub fn gated_total(price: Fixed, base: u32, bought: u32, count: u32) -> Option<i128> {
    (bought + 1..=bought + count).try_fold(0i128, |t, n| t.checked_add(gated_price(price, base, n)?))
}

fn gate_leaf(collection: &Address, g: &Gate) -> Hash {
    let leaves: Vec<Hash> = g.allowed.iter().map(|(k, n)| {
        let mut e = Encoder::new();
        e.bytes(b"objectvm.sale-allow-leaf.v1").bytes(k).u32(*n);
        e.leaf()
    }).collect();
    let mut e = Encoder::new();
    e.bytes(b"objectvm.sale-gate-leaf.v1").bytes(collection).u32(g.base).u32(leaves.len() as u32).bytes(&merkle_root(&leaves));
    e.leaf()
}

/// The gates' part of the market's leaf, joined only once a sale is gated.
pub(crate) fn gates_section(e: &mut Encoder, gates: &std::collections::BTreeMap<Address, Gate>) {
    let leaves: Vec<Hash> = gates.iter().map(|(c, g)| gate_leaf(c, g)).collect();
    e.bytes(b"objectvm.sale-gates.v1").u32(leaves.len() as u32).bytes(&merkle_root(&leaves));
}

pub(crate) fn encode_gates(e: &mut Encoder, gates: &std::collections::BTreeMap<Address, Gate>) {
    e.u32(gates.len() as u32);
    for (c, g) in gates {
        e.bytes(c).u32(g.base).u32(g.allowed.len() as u32);
        for (k, n) in &g.allowed {
            e.bytes(k).u32(*n);
        }
    }
}

pub(crate) fn decode_gates(d: &mut Decoder) -> Option<std::collections::BTreeMap<Address, Gate>> {
    let mut gates = std::collections::BTreeMap::new();
    for _ in 0..d.u32().ok()? {
        let c = d.hash().ok()?;
        let base = d.u32().ok()?;
        let mut allowed = std::collections::BTreeMap::new();
        for _ in 0..d.u32().ok()? {
            allowed.insert(d.hash().ok()?, d.u32().ok()?);
        }
        gates.insert(c, Gate { base, allowed });
    }
    Some(gates)
}

impl ObjectVm {
    /// Gate a sale (operator): before its first ticket, so nobody bought
    /// under rules it then changed. `base` may be reset until then.
    pub(crate) fn apply_gate_sale(&mut self, collection: &Address, base: u32) -> Result<Receipt, &'static str> {
        let (s, _) = self.sale(collection)?;
        if s.phase != Phase::Open || s.tickets > 0 {
            return Err("a sale is gated before its first ticket");
        }
        let g = self.market.sale_gates.entry(*collection).or_default();
        g.base = base;
        Ok(Receipt::SaleGated { collection: *collection, base })
    }

    /// Put accounts on a gated sale's list (operator), by their blinded keys.
    /// Add-only: an account already on it keeps its count.
    pub(crate) fn apply_allow_buyers(&mut self, collection: &Address, keys: &[Hash]) -> Result<Receipt, &'static str> {
        let (s, _) = self.sale(collection)?;
        // The list stays open through the open mint when it sets a price
        // there (§187): someone listed late still mints at it.
        let open_mint_list = s.phase == Phase::OpenMint && self.market.open_terms.contains_key(collection);
        if s.phase != Phase::Open && s.phase != Phase::Closed && s.phase != Phase::Drawn && !open_mint_list {
            return Err("the sale's list is closed");
        }
        if keys.is_empty() || keys.len() > MAX_ALLOW_PER_OP {
            return Err("add 1 to 256 accounts at a time");
        }
        let g = self.market.sale_gates.get_mut(collection).ok_or("that sale is not gated")?;
        let mut added = 0;
        for k in keys {
            if !g.allowed.contains_key(k) {
                g.allowed.insert(*k, 0);
                added += 1;
            }
        }
        Ok(Receipt::BuyersAllowed { collection: *collection, added })
    }

    /// Whether `who` may take part in a sale: anyone in an ungated one, a
    /// listed account in a gated one.
    pub(crate) fn sale_allows(&self, collection: &Address, who: &AccountId) -> bool {
        self.market.sale_gates.get(collection).is_none_or(|g| g.allowed.contains_key(&allow_key(collection, who)))
    }
}

// ---------------------------------------------------------------------------
// Open-mint terms (§187): a price for accounts on the sale's list (its gate),
// and a cap on how many items one account mints, counted by the same blinded
// key. Beside the sale, like the gate, so a sale without them hashes and
// saves exactly as before.

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct OpenTerms {
    /// What a listed account pays per item.
    pub listed_price: Fixed,
    /// Items one account mints at most.
    pub cap: u32,
    /// Items each account has minted, by `allow_key`.
    pub minted: std::collections::BTreeMap<Hash, u32>,
}

fn open_terms_leaf(collection: &Address, o: &OpenTerms) -> Hash {
    let leaves: Vec<Hash> = o.minted.iter().map(|(k, n)| {
        let mut e = Encoder::new();
        e.bytes(b"objectvm.open-minted-leaf.v1").bytes(k).u32(*n);
        e.leaf()
    }).collect();
    let mut e = Encoder::new();
    e.bytes(b"objectvm.open-terms-leaf.v1").bytes(collection).fixed(o.listed_price).u32(o.cap).u32(leaves.len() as u32).bytes(&merkle_root(&leaves));
    e.leaf()
}

/// The open-mint terms' part of the market's leaf, joined only once a sale has them.
pub(crate) fn open_terms_section(e: &mut Encoder, terms: &std::collections::BTreeMap<Address, OpenTerms>) {
    let leaves: Vec<Hash> = terms.iter().map(|(c, o)| open_terms_leaf(c, o)).collect();
    e.bytes(b"objectvm.open-terms.v1").u32(leaves.len() as u32).bytes(&merkle_root(&leaves));
}

pub(crate) fn encode_open_terms(e: &mut Encoder, terms: &std::collections::BTreeMap<Address, OpenTerms>) {
    e.u32(terms.len() as u32);
    for (c, o) in terms {
        e.bytes(c).fixed(o.listed_price).u32(o.cap).u32(o.minted.len() as u32);
        for (k, n) in &o.minted {
            e.bytes(k).u32(*n);
        }
    }
}

pub(crate) fn decode_open_terms(d: &mut Decoder) -> Option<std::collections::BTreeMap<Address, OpenTerms>> {
    let mut terms = std::collections::BTreeMap::new();
    for _ in 0..d.u32().ok()? {
        let c = d.hash().ok()?;
        let listed_price = d.fixed().ok()?;
        let cap = d.u32().ok()?;
        let mut minted = std::collections::BTreeMap::new();
        for _ in 0..d.u32().ok()? {
            minted.insert(d.hash().ok()?, d.u32().ok()?);
        }
        terms.insert(c, OpenTerms { listed_price, cap, minted });
    }
    Some(terms)
}

impl ObjectVm {
    /// Set a sale's open-mint terms (operator): before the first mint, so no
    /// one minted under terms that then changed. The listed price is at most
    /// the open price.
    pub(crate) fn apply_set_open_terms(&mut self, collection: &Address, listed_price: Fixed, cap: u32) -> Result<Receipt, &'static str> {
        let (s, _) = self.sale(collection)?;
        if s.phase == Phase::OpenMint && s.open_next > 0 {
            return Err("open-mint terms are set before the first mint");
        }
        if listed_price.0 <= 0 || listed_price.0 > s.open_price.0 {
            return Err("the listed price is above zero and at most the open price");
        }
        if cap == 0 {
            return Err("an account may mint at least one");
        }
        let o = self.market.open_terms.entry(*collection).or_default();
        o.listed_price = listed_price;
        o.cap = cap;
        Ok(Receipt::OpenTermsSet { collection: *collection, listed_price, cap })
    }
}
