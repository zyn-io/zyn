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
    Seal,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Receipt {
    Minted { commitment: Hash, position: u64 },
    Spent { spent: Vec<Hash>, created: Vec<Hash> },
    Deposited { commitment: Hash, position: u64 },
    Withdrawing { amount: Fixed, destination: Hash },
    Registered { account: AccountId },
    Listed { item: Hash },
    Delisted { item: Hash },
    Taken { item: Hash, created: Vec<Hash> },
    Refused(&'static str),
    Sealed(Checkpoint),
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
                .u64(x.since_epoch);
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

    fn account_leaf(id: &AccountId, spends: u64) -> Hash {
        let mut e = Encoder::new();
        e.bytes(b"objectvm.account.v1").bytes(id).u64(spends);
        e.leaf()
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
                Err(ObjectError::ZeroAmount) => {
                    return vec![Receipt::Refused("an output of nothing")]
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
        let paid: i128 = created
            .iter()
            .filter(|(_, n)| n.owner == offer.seller && n.asset == offer.price_asset)
            .map(|(_, n)| n.amount.0)
            .sum();
        if paid < offer.price_amount.0 {
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
        if *amount == Fixed::ZERO {
            return vec![Receipt::Refused("a withdrawal of nothing")];
        }
        if *destination == [0u8; 32] {
            return vec![Receipt::Refused("a withdrawal needs a destination")];
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
        let leaving = delta.entry(*asset).or_insert(0);
        *leaving = match leaving.checked_sub(amount.0) {
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
        self.exits.push(PendingExit {
            asset: *asset,
            amount: *amount,
            destination: *destination,
            since_epoch: self.epoch,
        });
        *self.accounts.entry(*holder).or_insert(0) += 1;

        vec![Receipt::Withdrawing {
            amount: *amount,
            destination: *destination,
        }]
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
                    Err(ObjectError::ZeroAmount) => {
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
                let supply = self.issued.entry(*asset).or_insert(Fixed::ZERO);
                *supply = match supply.0.checked_add(amount.0) {
                    Some(v) => Fixed(v),
                    None => return vec![Receipt::Refused("supply overflow")],
                };
                let position = self.commitments.append(c) as u64;
                self.live.insert(c, note);
                self.deposits.insert(*external_ref);
                self.accounts.entry(*owner).or_insert(0);
                vec![Receipt::Deposited {
                    commitment: c,
                    position,
                }]
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
                let note = match self.live.get(item) {
                    Some(n) => *n,
                    None => return vec![Receipt::Refused("no such live object")],
                };
                if &note.owner != seller {
                    return vec![Receipt::Refused("not the seller's object")];
                }
                if self.offers.contains_key(item) {
                    return vec![Receipt::Refused("already listed")];
                }
                if *price_amount == Fixed::ZERO {
                    return vec![Receipt::Refused("a listing needs a price")];
                }
                if *expires_at_epoch < self.epoch {
                    return vec![Receipt::Refused("already expired")];
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
            Op::Seal => {
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
        vec![
            self.header_leaf(),
            merkle_root(&self.account_leaves()),
            self.commitments.root(),
            self.nullifiers.root(),
            self.supply_leaf(),
            self.offers_leaf(),
        ]
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
            Op::PlaceOffer { seller, .. } | Op::CancelOffer { seller, .. } => vec![*seller],
            // Only the buyer signs a claim. The seller's consent was given
            // when they listed, and requiring it again would make every sale
            // need them online (§132.2).
            Op::TakeOffer { buyer, .. } => vec![*buyer],
            Op::Withdraw { holder, .. } => vec![*holder],
            Op::Register { account, .. } => vec![*account],
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
                e.u8(8).bytes(account).bytes(&program.0).bytes(viewing);
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
                .u64(x.since_epoch);
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
        Some(ObjectVm {
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
            .map(|(id, spends)| Self::account_leaf(id, *spends))
            .collect()
    }

    fn account_record(&self, id: &AccountId) -> Option<Vec<u8>> {
        let spends = self.accounts.get(id)?;
        let mut e = Encoder::new();
        e.bytes(b"objectvm.account.v1").bytes(id).u64(*spends);
        Some(e.finish().to_vec())
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
            let held = self.held(asset).ok_or("held overflow")?;
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

        // What the holder needs to spend it: the asset, the amount, and the
        // nonce. Not the owner — they know who they are, and a ciphertext
        // that names its recipient would say so to anyone who cracked one.
        let mut plain = Vec::with_capacity(80);
        plain.extend_from_slice(&asset);
        plain.extend_from_slice(&amount.0.to_be_bytes());
        plain.extend_from_slice(&nonce);
        let enc = seal(&to, ephemeral, &plain).map_err(|_| IngressError::AmountNotPositive)?;

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
