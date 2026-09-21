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

use std::collections::BTreeMap;

use zyn_vm::commit::{merkle_root, Encoder, Hash};
use zyn_vm::derive::Address;
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
        outputs: Vec<Note>,
    },
    Seal,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Receipt {
    Minted { commitment: Hash, position: u64 },
    Spent { spent: Vec<Hash>, created: Vec<Hash> },
    Refused(&'static str),
    Sealed(Checkpoint),
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
        outputs: &[Note],
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
            consumed.push(note);
        }

        // Outputs must be committable at all.
        let mut created: Vec<(Hash, Note)> = Vec::with_capacity(outputs.len());
        for note in outputs {
            match commitment(note) {
                Ok(c) => {
                    if self.live.contains_key(&c) {
                        return vec![Receipt::Refused("output already exists")];
                    }
                    created.push((c, *note))
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
                    e.bytes(&o.asset)
                        .fixed(o.amount)
                        .bytes(&o.owner)
                        .bytes(&o.nonce);
                }
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
                    outputs.push(Note {
                        asset: d.hash().ok()?,
                        amount: d.fixed().ok()?,
                        owner: d.account().ok()?,
                        nonce: d.hash().ok()?,
                    });
                }
                Some(Op::Spend {
                    spender,
                    program,
                    inputs,
                    outputs,
                })
            }
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
