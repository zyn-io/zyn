//! A collection's DAO (DECISIONS §172): member passes, proposals, votes and a
//! keyless treasury, all on the ledger.
//!
//! Membership is holding an unspent item of the collection. Every act a member
//! takes proves it by naming one of their live items, which the VM checks the
//! way it checks any input — present, theirs, of this collection — without
//! spending it. That proof is visible to the operator and replicas (tier 2,
//! §130) and to nobody else: the public sees tallies and pass hashes, never
//! which item voted or holds a pass.
//!
//! - **Passes.** A member registers the hash of a token they will hand to the
//!   members' chat. A pass is valid while the item it was made with is still
//!   live, so selling or moving the item retires its pass with no one acting.
//! - **Proposals.** A member proposes: the hash of the full text, and what the
//!   treasury should pay to whom, if anything. Enough items back it (the
//!   proposer's counts) and it goes to a vote; otherwise it lapses.
//! - **Votes.** One vote per item per proposal, by the item's own asset, so an
//!   item that changes hands mid-vote cannot vote twice. The vote closes after
//!   a set number of epochs and passes with more yes than no and enough items
//!   voting.
//! - **Treasury.** Once a collection has a DAO, the collection's creator shares
//!   — the treasury's cut of the sale and of each resale fee — accrue here
//!   instead of to the creator's account. Nothing leaves it except a passed
//!   proposal's payment, made at the seal that closes the vote. No key can
//!   move it.

use std::collections::{BTreeMap, BTreeSet};

use zyn_vm::commit::{merkle_root, Encoder, Hash};
use zyn_vm::derive::Address;
use zyn_vm::read::Decoder;
use zyn_vm::spec::AccountId;
use zyn_vm::Fixed;

use crate::market::{id, ITEM_UNIT};
use crate::{ObjectVm, Receipt};

/// Items one vote may count at once.
pub const MAX_VOTE_ITEMS: usize = 32;
/// A window lasts at most this many epochs: 30 days at the 15-second epochs
/// the node keeps while a proposal is open.
pub const MAX_VOTING_EPOCHS: u64 = 172_800;

pub const BACK: u8 = 0;
pub const YES: u8 = 1;
pub const NO: u8 = 2;

/// Where a proposal stands.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    /// Gathering backers until `until`.
    Backing { until: u64 },
    /// Open for votes until `until`.
    Voting { until: u64 },
    /// Passed; its payment, if any, was made.
    Passed,
    /// Passed, but the treasury could not cover its payment when it closed.
    Unfunded,
    /// Voted down, or too few voted.
    Failed,
    /// Too few backers in time.
    Lapsed,
}

impl State {
    fn tag(&self) -> (u8, u64) {
        match self {
            State::Backing { until } => (0, *until),
            State::Voting { until } => (1, *until),
            State::Passed => (2, 0),
            State::Unfunded => (3, 0),
            State::Failed => (4, 0),
            State::Lapsed => (5, 0),
        }
    }
    fn from_tag(t: u8, until: u64) -> Option<State> {
        Some(match t {
            0 => State::Backing { until },
            1 => State::Voting { until },
            2 => State::Passed,
            3 => State::Unfunded,
            4 => State::Failed,
            5 => State::Lapsed,
            _ => return None,
        })
    }
    pub fn word(&self) -> &'static str {
        match self {
            State::Backing { .. } => "backing",
            State::Voting { .. } => "voting",
            State::Passed => "passed",
            State::Unfunded => "unfunded",
            State::Failed => "failed",
            State::Lapsed => "lapsed",
        }
    }
}

/// A payment a proposal asks the treasury to make.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Payment {
    pub asset: Address,
    pub amount: Fixed,
    pub to: AccountId,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Proposal {
    /// The hash of the full text, which is published off the ledger.
    pub text: Hash,
    pub pay: Option<Payment>,
    pub state: State,
    /// Items that have backed, voted yes and voted no, by item asset.
    pub backers: BTreeSet<Address>,
    pub yes: BTreeSet<Address>,
    pub no: BTreeSet<Address>,
}

/// A collection's DAO.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Dao {
    /// Items that must back a proposal before it goes to a vote.
    pub backers: u32,
    /// Epochs a proposal gathers backers, then the same again to vote.
    pub voting_epochs: u64,
    /// Share of the collection's outstanding items that must vote, in bps.
    pub quorum_bps: u32,
    /// What the treasury holds, by asset.
    pub treasury: BTreeMap<Address, Fixed>,
    /// Member passes: the item commitment each was made with → the pass.
    pub passes: BTreeMap<Hash, Hash>,
    pub proposals: BTreeMap<u32, Proposal>,
}

/// The pass hash a token makes: what the chat is shown, never the token.
pub fn pass_of(token: &[u8]) -> Hash {
    id(b"objectvm.dao-pass.v1", &[token])
}

/// The claim source a proposal's payment is made under.
pub fn payment_source(collection: &Address, proposal: u32) -> Hash {
    id(b"objectvm.dao-pay.v1", &[collection, &proposal.to_be_bytes()])
}

fn set_root(s: &BTreeSet<Address>) -> Hash {
    merkle_root(&s.iter().copied().collect::<Vec<_>>())
}

fn proposal_leaf(n: u32, p: &Proposal) -> Hash {
    let (t, until) = p.state.tag();
    let mut e = Encoder::new();
    e.bytes(b"objectvm.dao-proposal.v1").u32(n).bytes(&p.text);
    match &p.pay {
        None => {
            e.u8(0);
        }
        Some(x) => {
            e.u8(1).bytes(&x.asset).fixed(x.amount).bytes(&x.to);
        }
    }
    e.u8(t).u64(until).bytes(&set_root(&p.backers)).bytes(&set_root(&p.yes)).bytes(&set_root(&p.no));
    e.leaf()
}

fn dao_leaf(collection: &Address, d: &Dao) -> Hash {
    let treasury: Vec<Hash> = d.treasury.iter().map(|(a, v)| {
        let mut e = Encoder::new();
        e.bytes(b"objectvm.dao-treasury.v1").bytes(a).fixed(*v);
        e.leaf()
    }).collect();
    let passes: Vec<Hash> = d.passes.iter().map(|(c, p)| {
        let mut e = Encoder::new();
        e.bytes(b"objectvm.dao-pass-leaf.v1").bytes(c).bytes(p);
        e.leaf()
    }).collect();
    let proposals: Vec<Hash> = d.proposals.iter().map(|(n, p)| proposal_leaf(*n, p)).collect();
    let mut e = Encoder::new();
    e.bytes(b"objectvm.dao.v1")
        .bytes(collection)
        .u32(d.backers)
        .u64(d.voting_epochs)
        .u32(d.quorum_bps)
        .bytes(&merkle_root(&treasury))
        .bytes(&merkle_root(&passes))
        .bytes(&merkle_root(&proposals));
    e.leaf()
}

/// The DAOs' part of the market's leaf, joined only once a DAO is set.
pub(crate) fn section(e: &mut Encoder, daos: &BTreeMap<Address, Dao>) {
    let leaves: Vec<Hash> = daos.iter().map(|(c, d)| dao_leaf(c, d)).collect();
    e.bytes(b"objectvm.daos.v1").u32(leaves.len() as u32).bytes(&merkle_root(&leaves));
}

fn put_set(e: &mut Encoder, s: &BTreeSet<Address>) {
    e.u32(s.len() as u32);
    for a in s {
        e.bytes(a);
    }
}

fn get_set(d: &mut Decoder) -> Option<BTreeSet<Address>> {
    let mut s = BTreeSet::new();
    for _ in 0..d.u32().ok()? {
        s.insert(d.hash().ok()?);
    }
    Some(s)
}

pub(crate) fn encode(e: &mut Encoder, daos: &BTreeMap<Address, Dao>) {
    e.u32(daos.len() as u32);
    for (c, d) in daos {
        e.bytes(c).u32(d.backers).u64(d.voting_epochs).u32(d.quorum_bps);
        e.u32(d.treasury.len() as u32);
        for (a, v) in &d.treasury {
            e.bytes(a).fixed(*v);
        }
        e.u32(d.passes.len() as u32);
        for (c, p) in &d.passes {
            e.bytes(c).bytes(p);
        }
        e.u32(d.proposals.len() as u32);
        for (n, p) in &d.proposals {
            e.u32(*n).bytes(&p.text);
            match &p.pay {
                None => {
                    e.u8(0);
                }
                Some(x) => {
                    e.u8(1).bytes(&x.asset).fixed(x.amount).bytes(&x.to);
                }
            }
            let (t, until) = p.state.tag();
            e.u8(t).u64(until);
            put_set(e, &p.backers);
            put_set(e, &p.yes);
            put_set(e, &p.no);
        }
    }
}

pub(crate) fn decode(d: &mut Decoder) -> Option<BTreeMap<Address, Dao>> {
    let mut daos = BTreeMap::new();
    for _ in 0..d.u32().ok()? {
        let c = d.hash().ok()?;
        let mut dao = Dao { backers: d.u32().ok()?, voting_epochs: d.u64().ok()?, quorum_bps: d.u32().ok()?, ..Default::default() };
        for _ in 0..d.u32().ok()? {
            dao.treasury.insert(d.hash().ok()?, d.fixed().ok()?);
        }
        for _ in 0..d.u32().ok()? {
            dao.passes.insert(d.hash().ok()?, d.hash().ok()?);
        }
        for _ in 0..d.u32().ok()? {
            let n = d.u32().ok()?;
            let text = d.hash().ok()?;
            let pay = match d.u8().ok()? {
                0 => None,
                1 => Some(Payment { asset: d.hash().ok()?, amount: d.fixed().ok()?, to: d.account().ok()? }),
                _ => return None,
            };
            let t = d.u8().ok()?;
            let until = d.u64().ok()?;
            let state = State::from_tag(t, until)?;
            dao.proposals.insert(n, Proposal { text, pay, state, backers: get_set(d)?, yes: get_set(d)?, no: get_set(d)? });
        }
        daos.insert(c, dao);
    }
    Some(daos)
}

impl ObjectVm {
    /// The item asset behind a member's live item of `collection`, checked as
    /// an input is — present, theirs, one whole item of that collection — and
    /// not spent.
    fn member_item(&self, holder: &AccountId, collection: &Address, c: &Hash) -> Result<Address, &'static str> {
        let note = self.live.get(c).ok_or("no such live item")?;
        if &note.owner != holder {
            return Err("not the holder's item");
        }
        if self.market.items.get(&note.asset).map(|i| &i.collection) != Some(collection) {
            return Err("not an item of that collection");
        }
        if note.amount != ITEM_UNIT {
            return Err("an item is one whole unit");
        }
        Ok(note.asset)
    }

    /// Set (or retune) a collection's DAO. Operator only. From the first time,
    /// the collection's creator shares go to its treasury.
    pub(crate) fn apply_set_dao(&mut self, collection: &Address, backers: u32, voting_epochs: u64, quorum_bps: u32) -> Result<Receipt, &'static str> {
        if !self.market.collections.contains_key(collection) {
            return Err("that is not a registered collection");
        }
        if backers == 0 || voting_epochs == 0 || voting_epochs > MAX_VOTING_EPOCHS || quorum_bps > 10_000 {
            return Err("backers and voting time must be positive, voting at most 172,800 epochs, quorum at most 100%");
        }
        let d = self.market.daos.entry(*collection).or_default();
        d.backers = backers;
        d.voting_epochs = voting_epochs;
        d.quorum_bps = quorum_bps;
        Ok(Receipt::DaoSet { collection: *collection })
    }

    /// Register a member pass: `pass` is valid while `item` stays live.
    /// Re-registering with the same item replaces its pass.
    pub(crate) fn apply_member_pass(&mut self, holder: &AccountId, collection: &Address, item: &Hash, pass: &Hash) -> Result<Receipt, &'static str> {
        if !self.market.daos.contains_key(collection) {
            return Err("that collection has no DAO");
        }
        if *pass == [0u8; 32] {
            return Err("a pass cannot be zero");
        }
        self.member_item(holder, collection, item)?;
        let live = &self.live;
        let d = self.market.daos.get_mut(collection).expect("checked above");
        // Passes whose items have moved are retired here, so the set holds
        // only what can still be valid.
        d.passes.retain(|c, _| live.contains_key(c));
        if d.passes.iter().any(|(c, p)| p == pass && c != item) {
            return Err("that pass is another item's");
        }
        d.passes.insert(*item, *pass);
        *self.accounts.entry(*holder).or_insert(0) += 1;
        Ok(Receipt::PassSet { collection: *collection })
    }

    /// Whether `pass` is a member pass of `collection` whose item is still live.
    pub fn pass_valid(&self, collection: &Address, pass: &Hash) -> bool {
        self.market.daos.get(collection).is_some_and(|d| d.passes.iter().any(|(c, p)| p == pass && self.live.contains_key(c)))
    }

    /// Propose: the proposer's item backs it first.
    pub(crate) fn apply_propose(&mut self, proposer: &AccountId, collection: &Address, item: &Hash, text: &Hash, pay: &Option<Payment>) -> Result<Receipt, &'static str> {
        let asset = self.member_item(proposer, collection, item)?;
        if let Some(p) = pay {
            if p.amount.0 <= 0 {
                return Err("a payment must be worth something");
            }
        }
        let epoch = self.epoch;
        let d = self.market.daos.get_mut(collection).ok_or("that collection has no DAO")?;
        let n = d.proposals.keys().next_back().map_or(1, |n| n + 1);
        let mut backers = BTreeSet::new();
        backers.insert(asset);
        let state = if d.backers <= 1 { State::Voting { until: epoch + d.voting_epochs } } else { State::Backing { until: epoch + d.voting_epochs } };
        d.proposals.insert(n, Proposal { text: *text, pay: *pay, state, backers, yes: BTreeSet::new(), no: BTreeSet::new() });
        *self.accounts.entry(*proposer).or_insert(0) += 1;
        Ok(Receipt::Proposed { collection: *collection, id: n })
    }

    /// Back or vote with items: each item counts once per proposal.
    pub(crate) fn apply_vote(&mut self, voter: &AccountId, collection: &Address, proposal: u32, items: &[Hash], choice: u8) -> Result<Receipt, &'static str> {
        if items.is_empty() || items.len() > MAX_VOTE_ITEMS {
            return Err("vote with 1 to 32 items");
        }
        let mut assets = Vec::with_capacity(items.len());
        for c in items {
            let a = self.member_item(voter, collection, c)?;
            if assets.contains(&a) {
                return Err("the same item twice");
            }
            assets.push(a);
        }
        let epoch = self.epoch;
        let d = self.market.daos.get_mut(collection).ok_or("that collection has no DAO")?;
        let need = d.backers as usize;
        let voting = d.voting_epochs;
        let p = d.proposals.get_mut(&proposal).ok_or("no such proposal")?;
        // Items that have already acted on this proposal are passed over, so a
        // wallet can act with every item it holds without knowing which voted
        // (that is not public); only an act that counts nothing new is refused.
        let counted = match (choice, p.state) {
            (BACK, State::Backing { until }) if epoch < until => {
                let new: Vec<Address> = assets.iter().copied().filter(|a| !p.backers.contains(a)).collect();
                if new.is_empty() {
                    return Err("every item has already backed this proposal");
                }
                p.backers.extend(new.iter().copied());
                // Enough backers: the vote opens now, for its full time.
                if p.backers.len() >= need {
                    p.state = State::Voting { until: epoch + voting };
                }
                new.len()
            }
            (YES | NO, State::Voting { until }) if epoch < until => {
                let new: Vec<Address> = assets.iter().copied().filter(|a| !p.yes.contains(a) && !p.no.contains(a)).collect();
                if new.is_empty() {
                    return Err("every item has already voted on this proposal");
                }
                let side = if choice == YES { &mut p.yes } else { &mut p.no };
                side.extend(new.iter().copied());
                new.len()
            }
            (BACK, _) => return Err("this proposal is not gathering backers"),
            (YES | NO, _) => return Err("this proposal is not open for votes"),
            _ => return Err("back, yes or no"),
        };
        *self.accounts.entry(*voter).or_insert(0) += 1;
        Ok(Receipt::Voted { collection: *collection, id: proposal, items: counted as u32 })
    }

    /// At the seal: lapse proposals that found too few backers in time, and
    /// close votes whose time is up — paying a passed proposal's payment from
    /// the treasury if it can.
    pub(crate) fn settle_daos(&mut self) {
        // Acts are taken while `epoch < until`, so the seal of epoch
        // `until - 1` is the last one before none can be: close there.
        let closing = self.epoch + 1;
        let collections: Vec<Address> = self.market.daos.keys().copied().collect();
        for c in collections {
            let outstanding = self.market.collections.get(&c).map_or(0, |t| t.outstanding) as u128;
            let mut payments: Vec<(u32, Payment)> = Vec::new();
            let d = self.market.daos.get_mut(&c).expect("listed above");
            let quorum = (outstanding * d.quorum_bps as u128).div_ceil(10_000);
            for (n, p) in d.proposals.iter_mut() {
                match p.state {
                    State::Backing { until } if until <= closing => p.state = State::Lapsed,
                    State::Voting { until } if until <= closing => {
                        let turnout = (p.yes.len() + p.no.len()) as u128;
                        if p.yes.len() > p.no.len() && turnout >= quorum.max(1) {
                            match p.pay {
                                None => p.state = State::Passed,
                                Some(x) => {
                                    let held = d.treasury.get(&x.asset).copied().unwrap_or(Fixed::ZERO);
                                    if held.0 >= x.amount.0 {
                                        d.treasury.insert(x.asset, Fixed(held.0 - x.amount.0));
                                        payments.push((*n, x));
                                        p.state = State::Passed;
                                    } else {
                                        p.state = State::Unfunded;
                                    }
                                }
                            }
                        } else {
                            p.state = State::Failed;
                        }
                    }
                    _ => {}
                }
            }
            d.treasury.retain(|_, v| v.0 > 0);
            for (n, x) in payments {
                self.market.pay(&payment_source(&c, n), 0, x.to, x.asset, x.amount);
            }
        }
    }

    /// Where a collection's creator share goes: its DAO's treasury once it has
    /// one, else a claim for the creator.
    pub(crate) fn pay_creator_share(&mut self, collection: &Address, source: &Hash, creator: AccountId, asset: Address, amount: Fixed) {
        if amount.0 <= 0 {
            return;
        }
        match self.market.daos.get_mut(collection) {
            Some(d) => {
                let v = d.treasury.entry(asset).or_insert(Fixed::ZERO);
                v.0 = v.0.saturating_add(amount.0);
            }
            None => self.market.pay(source, 0, creator, asset, amount),
        }
    }
}

/// What every DAO's treasury holds of `asset`, for conservation.
pub(crate) fn holdings(daos: &BTreeMap<Address, Dao>, asset: &Address) -> Option<i128> {
    daos.values().try_fold(0i128, |t, d| t.checked_add(d.treasury.get(asset).map_or(0, |v| v.0)))
}
