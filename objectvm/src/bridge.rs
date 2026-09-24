//! Assets bridged from other chains beside ZEC — SOL.zy, ETH.zy and the
//! tokens that follow (DECISIONS §146).
//!
//! ZEC.zy keeps its own path: a per-account shielded deposit address, credited
//! straight to its owner. Other chains have no shielded address to give each
//! account, so a deposit there carries a **commitment** instead — `H(account,
//! secret)` from the depositor's Nap, in a Solana memo or an Ethereum call —
//! and names no Zyn account at all:
//!
//! * `RegisterBridged` (operator) records an origin — the network, the token
//!   there (empty for the network's own coin) and its decimals — under the
//!   address §66.13 derives from those public facts. Only registered assets
//!   take committed deposits, and the origin registry is the frozen one.
//! * `DepositCommitted` (operator) records one deposit on the other chain,
//!   once by its reference: the gross amount is issued (the vault holds it),
//!   the bridge fee goes to the pot, and the rest waits under
//!   `H(commitment, asset)`. Deposits with the same commitment add up.
//! * `ClaimDeposit` (the depositor) reveals the secret from the committed
//!   account and takes everything waiting as one note. A revealed secret is
//!   useless to anyone else: the commitment binds the account.
//!
//! Leaving is the ordinary `Withdraw`; which vault pays it is the operator's
//! settler for that chain. What waits unclaimed is in `Market::holdings`, so
//! conservation holds throughout.

use zyn_vm::commit::{Encoder, Hash};
use zyn_vm::derive::{bridged_address, Address, ADDRESS_SCOPE_V1};
use zyn_vm::object::{commitment, Note};
use zyn_vm::read::Decoder;
use zyn_vm::spec::AccountId;
use zyn_vm::Fixed;

use crate::{market, ObjectVm, Output, Receipt};

/// An origin: where a bridged asset really lives.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Origin {
    /// `solana`, `solana-devnet`, `eip155:<chain id>`, … (§66.13).
    pub network: Vec<u8>,
    /// Empty for the network's own coin; the 32-byte mint or 20-byte contract
    /// for a token issued there.
    pub asset: Vec<u8>,
    /// The origin's decimals, for wallets to scale by. Amounts on Zyn are
    /// always 18-decimal fixed point.
    pub decimals: u8,
}

/// The frozen origin registry (§66.13), as the AMM chain enforced it.
pub fn valid_origin(network: &[u8], asset: &[u8]) -> bool {
    match network {
        b"solana" | b"solana-devnet" => asset.is_empty() || asset.len() == 32,
        b"zcash" | b"zcash-test" | b"bitcoin" | b"bitcoin-test" | b"bitcoin-signet" => asset.is_empty(),
        _ => {
            let Some(chain) = network.strip_prefix(b"eip155:") else {
                return false;
            };
            !chain.is_empty() && chain.iter().all(u8::is_ascii_digit) && (chain.len() == 1 || chain[0] != b'0') && (asset.is_empty() || asset.len() == 20)
        }
    }
}

/// The address an origin is bridged under: public facts in, one address out.
pub fn asset_of(network: &[u8], asset: &[u8]) -> Address {
    bridged_address(ADDRESS_SCOPE_V1, network, asset)
}

fn id(domain: &[u8], parts: &[&[u8]]) -> Hash {
    let mut e = Encoder::new();
    e.bytes(domain);
    for p in parts {
        e.bytes(p);
    }
    e.leaf()
}

/// What a deposit on another chain carries: the depositor's account and a
/// secret only their wallet holds, hashed. It names nobody.
pub fn deposit_commitment(account: &AccountId, secret: &Hash) -> Hash {
    id(b"objectvm.deposit.v1", &[account, secret])
}

/// Where a committed deposit waits: per commitment and asset.
pub fn deposit_key(commitment: &Hash, asset: &Address) -> Hash {
    id(b"objectvm.deposit-claim.v1", &[commitment, asset])
}

/// One committed deposit waiting to be claimed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Waiting {
    pub asset: Address,
    pub amount: Fixed,
}

pub(crate) fn origin_leaf(asset: &Address, o: &Origin) -> Hash {
    let mut e = Encoder::new();
    e.bytes(b"objectvm.bridged-leaf.v1").bytes(asset).u32(o.network.len() as u32).bytes(&o.network).u32(o.asset.len() as u32).bytes(&o.asset).u8(o.decimals);
    e.leaf()
}

pub(crate) fn waiting_leaf(key: &Hash, w: &Waiting) -> Hash {
    let mut e = Encoder::new();
    e.bytes(b"objectvm.deposit-waiting-leaf.v1").bytes(key).bytes(&w.asset).fixed(w.amount);
    e.leaf()
}

pub(crate) fn encode(e: &mut Encoder, bridged: &std::collections::BTreeMap<Address, Origin>, waiting: &std::collections::BTreeMap<Hash, Waiting>) {
    e.u32(bridged.len() as u32);
    for (a, o) in bridged {
        e.bytes(a).u32(o.network.len() as u32).bytes(&o.network).u32(o.asset.len() as u32).bytes(&o.asset).u8(o.decimals);
    }
    e.u32(waiting.len() as u32);
    for (k, w) in waiting {
        e.bytes(k).bytes(&w.asset).fixed(w.amount);
    }
}

pub(crate) type BridgeState = (std::collections::BTreeMap<Address, Origin>, std::collections::BTreeMap<Hash, Waiting>);

pub(crate) fn decode(d: &mut Decoder) -> Option<BridgeState> {
    let mut bridged = std::collections::BTreeMap::new();
    for _ in 0..d.u32().ok()? {
        let a = d.hash().ok()?;
        let n = d.u32().ok()? as usize;
        if n > 64 {
            return None;
        }
        let network = d.take_bytes(n).ok()?.to_vec();
        let m = d.u32().ok()? as usize;
        if m > 32 {
            return None;
        }
        let asset = d.take_bytes(m).ok()?.to_vec();
        bridged.insert(a, Origin { network, asset, decimals: d.u8().ok()? });
    }
    let mut waiting = std::collections::BTreeMap::new();
    for _ in 0..d.u32().ok()? {
        let k = d.hash().ok()?;
        waiting.insert(k, Waiting { asset: d.hash().ok()?, amount: d.fixed().ok()? });
    }
    Some((bridged, waiting))
}

impl ObjectVm {
    pub(crate) fn apply_register_bridged(&mut self, network: &[u8], origin: &[u8], decimals: u8) -> Result<Receipt, &'static str> {
        if !valid_origin(network, origin) {
            return Err("that origin is not in the registry");
        }
        if decimals > 36 {
            return Err("more decimals than any chain uses");
        }
        let asset = asset_of(network, origin);
        if self.market.bridged.contains_key(&asset) {
            return Err("that asset is already registered");
        }
        if self.market.is_derived(&asset) {
            return Err("that address is already something else");
        }
        self.market.bridged.insert(asset, Origin { network: network.to_vec(), asset: origin.to_vec(), decimals });
        Ok(Receipt::BridgedRegistered { asset })
    }

    pub(crate) fn apply_deposit_committed(&mut self, asset: &Address, gross: Fixed, external_ref: &Hash, commitment: &Hash) -> Result<Receipt, &'static str> {
        if !self.market.bridged.contains_key(asset) {
            return Err("that asset is not bridged here");
        }
        if gross.0 <= 0 {
            return Err("a deposit must be of something");
        }
        if *external_ref == [0; 32] || *commitment == [0; 32] {
            return Err("a deposit names the transaction behind it and a commitment");
        }
        if self.deposits.contains(external_ref) {
            return Err("that deposit was already credited");
        }
        let fee = market::fee_of(gross, self.market.bridge_bps()).ok_or("fee overflow")?;
        let net = Fixed(gross.0.checked_sub(fee.0).ok_or("fee overflow")?);
        let key = deposit_key(commitment, asset);
        let before = self.market.waiting.get(&key).map_or(0, |w| w.amount.0);
        let after = before.checked_add(net.0).ok_or("deposit overflow")?;
        let supply = self.issued.get(asset).map_or(0, |s| s.0).checked_add(gross.0).ok_or("supply overflow")?;
        self.market.pol.get(asset).map_or(0, |p| p.0).checked_add(fee.0).ok_or("pot overflow")?;
        self.issued.insert(*asset, Fixed(supply));
        self.deposits.insert(*external_ref);
        let _ = self.market.to_pol(*asset, fee);
        self.market.waiting.insert(key, Waiting { asset: *asset, amount: Fixed(after) });
        Ok(Receipt::DepositWaiting { key, amount: net })
    }

    pub(crate) fn apply_claim_deposit(&mut self, owner: &AccountId, secret: &Hash, asset: &Address, out: &Output) -> Result<Receipt, &'static str> {
        let key = deposit_key(&deposit_commitment(owner, secret), asset);
        let w = *self.market.waiting.get(&key).ok_or("nothing is waiting for this account")?;
        let note: &Note = &out.note;
        if note.asset != w.asset || note.amount != w.amount || &note.owner != owner {
            return Err("the note must be everything waiting, to the depositor");
        }
        if out.enc.ciphertext.is_empty() {
            return Err("a deposit needs an encrypted note");
        }
        let c = commitment(note).map_err(|_| "a deposit needs a nonce and a value")?;
        if self.live.contains_key(&c) {
            return Err("that object already exists");
        }
        self.market.waiting.remove(&key);
        let position = self.commitments.append(c) as u64;
        self.live.insert(c, *note);
        self.accounts.entry(*owner).or_insert(0);
        Ok(Receipt::Deposited { commitment: c, position })
    }
}
