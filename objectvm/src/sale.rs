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
//! * **Burn tickets** (`SetVoucherCap`, `PostVoucher`, `ClaimVoucherItems`,
//!   DECISIONS §145): the operator records a burn of a promotional token on
//!   another chain, once, and it becomes tickets in the same list, under ids
//!   derived from a commitment the burner put in the burn's memo (their Zyn
//!   account and a secret). They pay nothing in, so a sale caps how many there
//!   may be; their winners claim by revealing the secret from that account and
//!   pay the claim price like anyone.
//!
//! The sale holds no value of its own: every payment is in the floor or a
//! claim the moment it is made, so conservation needs nothing new.

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

impl ObjectVm {
    fn sale(&self, collection: &Address) -> Result<(Sale, CollectionTerms), &'static str> {
        let s = *self.market.sales.get(collection).ok_or("that collection has no sale")?;
        let t = *self.market.collections.get(collection).ok_or("register the collection first")?;
        Ok((s, t))
    }

    /// Take `amount` of the collection's quote asset from `who`: the treasury's
    /// share owed to the creator, the rest into the floor.
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
        let (s, t) = self.sale(collection)?;
        let f = self.plan_funding(who, inputs, change, &[(t.quote, amount)])?;
        let treasury = zyn_vm::fixed::mul_div(amount.0, s.treasury_bps as i128, BPS).ok_or("payment overflow")?;
        self.apply_funding(who, inputs, f);
        let t = *self.market.collections.get(collection).expect("checked above");
        self.market.collections.insert(*collection, CollectionTerms { floor: Fixed(t.floor.0 + amount.0 - treasury), ..t });
        self.market.pay(&treasury_source(collection, payment), 0, t.creator, t.quote, Fixed(treasury));
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
            if s.tickets > 0 || s.phase != Phase::Open {
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
        let total = s.ticket_price.0.checked_mul(count as i128).ok_or("payment overflow")?;
        self.take_sale_payment(buyer, collection, &first, Fixed(total), inputs, change)?;
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
        self.take_sale_payment(buyer, collection, &payment, s.open_price, inputs, change)?;
        self.mint_checked(collection, serial, item, c)?;
        self.market.sales.get_mut(collection).expect("checked above").open_next = s.open_next + 1;
        Ok(Receipt::ItemsClaimed { created: vec![c] })
    }
}
