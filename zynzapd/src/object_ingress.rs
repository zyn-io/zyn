//! Confirmed ZEC becomes an object on the ledger.
//!
//! Two halves of the bridge existed and did not touch. [`Watcher`] reads
//! Zcash and reports deposits that are confirmed to the configured depth;
//! [`ingress::credit`] builds the intent that turns one deposit into an
//! object. Nothing joined them, so every deposit the object ledger had ever
//! seen was one a test invented (A116).
//!
//! Joining them is not a function call, because the join has to survive a
//! crash.
//!
//! # Why the nonce is derived and not random
//!
//! A deposit's object commits to a nonce, and that nonce must be:
//!
//! - **unguessable**, or a stranger who knows the account and can guess the
//!   amount — both public, one from the memo and one from the Zcash
//!   transaction — recomputes the commitment and learns which object on the
//!   ledger is that deposit; and
//! - **the same on every attempt**, or a bridge that dies between building an
//!   intent and learning whether it landed rebuilds a *different* object for
//!   the same deposit on restart.
//!
//! Random satisfies the first and fails the second. The failure is not a
//! retry that works: the VM dedupes on the Zcash txid (`vm.deposits`), so the
//! second intent is refused as `AlreadyCredited`, and the bridge is left
//! holding an object the holder's wallet will never find — because the wallet
//! scans for the commitment the *first* intent published, and the bridge has
//! forgotten the nonce that produced it. The money is in the vault, the
//! credit is on the ledger, and the two cannot be connected by anyone.
//!
//! So both secrets are derived from the bridge's seed and the txid:
//!
//! ```text
//!   nonce      HKDF(seed, "zyn.ingress.nonce.v1",     "nonce"     || txid)
//!   ephemeral  HKDF(seed, "zyn.ingress.ephemeral.v1", "ephemeral" || txid)
//! ```
//!
//! Deterministic given the seed, unguessable without it. A restarted bridge
//! rebuilds the identical intent, so resubmitting is free and being refused
//! as a duplicate is proof the work was already done rather than a loss.
//!
//! Keyed on the txid alone, deliberately. The txid is exactly what the VM
//! dedupes on, so anything else in the derivation would let one deposit have
//! two valid nonces — which is the failure this exists to prevent.
//!
//! # The seed is not the wallet seed
//!
//! It is the bridge operator's, and anyone holding it can recompute every
//! deposit commitment the bridge has ever made and link each to its Zcash
//! transaction. It does not let them *spend* anything — spending needs the
//! holder's key — but it undoes the privacy of every deposit at once. It
//! belongs where the vault's other secrets are, not in a config file.

use hkdf::Hkdf;
use sha2::Sha256;

use objectvm::{ingress, ObjectVm, Op};
use zyn_custody::watcher::{AssetId, WatcherAction};
use zyn_vm::note_enc::StaticSecret;
use zyn_vm::spec::AccountId;
use zyn_vm::Fixed;

/// Finds the nonce a deposit's object commits to.
const NONCE_SALT: &[u8] = b"zyn.ingress.nonce.v1";
const NONCE_INFO: &[u8] = b"nonce";

/// Finds the key the deposit's note was sealed with.
const EPHEMERAL_SALT: &[u8] = b"zyn.ingress.ephemeral.v1";
const EPHEMERAL_INFO: &[u8] = b"ephemeral";

fn derive(seed: &[u8], salt: &[u8], info: &[u8], txid: &[u8; 32]) -> [u8; 32] {
    let mut labelled = info.to_vec();
    labelled.extend_from_slice(txid);
    let mut out = [0u8; 32];
    Hkdf::<Sha256>::new(Some(salt), seed)
        .expand(&labelled, &mut out)
        .expect("32 bytes is within HKDF's output limit");
    out
}

/// The nonce this deposit's object commits to. Same every time, for this
/// seed and this Zcash transaction.
pub fn deposit_nonce(seed: &[u8], txid: &[u8; 32]) -> [u8; 32] {
    derive(seed, NONCE_SALT, NONCE_INFO, txid)
}

/// The key this deposit's note is sealed with. Distinct per deposit, so two
/// deposits to one account do not share an ephemeral key and cannot be linked
/// by it.
pub fn deposit_ephemeral(seed: &[u8], txid: &[u8; 32]) -> StaticSecret {
    StaticSecret::from(derive(seed, EPHEMERAL_SALT, EPHEMERAL_INFO, txid))
}

/// What the bridge should do about one thing the watcher reported.
#[derive(Clone, PartialEq, Debug)]
pub enum Planned {
    /// Submit this. It credits a confirmed deposit.
    Submit(Box<Op>),
    /// The account has never said where to pay it, so an object made for it
    /// would be one nobody can open. The ZEC is in the vault and stays there;
    /// this deposit is *pending*, not dropped, and becomes creditable the
    /// moment the holder registers. Nothing expires it.
    AwaitingViewingKey {
        account: AccountId,
        amount: Fixed,
        txid: [u8; 32],
    },
    /// Already on the ledger. The normal result of a restart, and not a
    /// problem: the intent was built from the same seed and txid, so the
    /// object already published is the one this would have made.
    AlreadyCredited { txid: [u8; 32] },
    /// The watcher's balance report. It has no home on the object ledger —
    /// there is no attest intent — so it is handed back for the caller to
    /// check against [`solvent`] rather than quietly discarded.
    Attestation { asset: Option<AssetId>, observed: Fixed },
    /// A deposit the watcher reported with a non-positive amount. A chain
    /// view that produces one is wrong, and crediting it would mint.
    Rejected { txid: [u8; 32], why: &'static str },
}

/// Turn what the watcher saw into what the ledger should be told.
///
/// Takes the actions in the order the watcher produced them, which is the
/// order they must be applied in: an attestation always precedes the credits
/// it covers.
///
/// `zec` is the asset id for the bridge's own asset — the watcher reports
/// `asset: None` for it, because on the custody side it is *the* asset and
/// needs no name.
pub fn plan(
    vm: &ObjectVm,
    seed: &[u8],
    zec: AssetId,
    actions: &[WatcherAction],
) -> Vec<Planned> {
    actions
        .iter()
        .map(|a| match a {
            WatcherAction::Attest { observed } => Planned::Attestation {
                asset: None,
                observed: *observed,
            },
            WatcherAction::AttestAsset { asset, observed } => Planned::Attestation {
                asset: Some(*asset),
                observed: *observed,
            },
            WatcherAction::Credit {
                account,
                amount,
                external_ref,
                asset,
                ..
            } => {
                let asset = asset.unwrap_or(zec);
                let txid = *external_ref;
                match ingress::credit(
                    vm,
                    asset,
                    *amount,
                    *account,
                    txid,
                    deposit_nonce(seed, &txid),
                    deposit_ephemeral(seed, &txid),
                ) {
                    Ok(op) => Planned::Submit(Box::new(op)),
                    Err(ingress::IngressError::NoViewingKey) => Planned::AwaitingViewingKey {
                        account: *account,
                        amount: *amount,
                        txid,
                    },
                    Err(ingress::IngressError::AlreadyCredited) => {
                        Planned::AlreadyCredited { txid }
                    }
                    Err(ingress::IngressError::AmountNotPositive) => Planned::Rejected {
                        txid,
                        why: "the watcher reported a deposit of no value",
                    },
                }
            }
        })
        .collect()
}

/// Is the vault holding at least what the ledger says it issued?
///
/// The only question an attestation can answer here. `observed` is what the
/// watcher read from Zcash at a confirmed height; `issued` is what the object
/// ledger has created. Holding *more* is fine and normal — a top-up to the
/// all-zero account raises the balance and credits nobody. Holding less means
/// objects exist that nothing backs, and no intent repairs that.
pub fn solvent(vm: &ObjectVm, asset: &AssetId, observed: Fixed) -> bool {
    let issued = vm.issued.get(asset).copied().unwrap_or(Fixed::ZERO);
    observed.0 >= issued.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use objectvm::{Limits, Receipt};
    use zyn_vm::note_enc::viewing_public;
    use zyn_vm::program;
    use zyn_vm::scan::{PublishedOutput, Scanner};
    use zyn_vm::spec::MicrochainVm;

    const SEED: &[u8] = b"the bridge's seed, which is not a wallet seed";
    /// A nullifier key for tests. Real ones come from `object_keys`.
    const NKEY: [u8; 32] = [0x31; 32];
    const ZEC: AssetId = [0x7a; 32];
    const ALICE: AccountId = [0xA1; 32];

    fn alice_view() -> StaticSecret {
        StaticSecret::from([0x11; 32])
    }

    fn vm_with_alice_registered() -> ObjectVm {
        let mut vm = ObjectVm::genesis(212, Limits { max_io: 8 });
        vm.apply(
            1,
            &Op::Register {
                account: ALICE,
                program: program::register_viewing_key(),
                nullifier_commitment: ObjectVm::nullifier_commitment(&NKEY),
                viewing: viewing_public(&alice_view()),
            },
        );
        vm
    }

    fn credit_of(txid: [u8; 32], amount: i64) -> WatcherAction {
        WatcherAction::Credit {
            account: ALICE,
            amount: Fixed::whole(amount),
            index: 0,
            external_ref: txid,
            asset: None,
        }
    }

    /// The whole point: a deposit Zcash confirmed becomes an object its
    /// holder — and only its holder — can find.
    #[test]
    fn a_confirmed_deposit_becomes_an_object_its_holder_can_find() {
        let mut vm = vm_with_alice_registered();
        let planned = plan(&vm, SEED, ZEC, &[credit_of([0xDD; 32], 7)]);

        let Planned::Submit(op) = &planned[0] else {
            panic!("expected an intent, got {:?}", planned[0]);
        };
        let receipts = vm.apply(2, op);
        let Receipt::Deposited { commitment, .. } = receipts[0] else {
            panic!("the ledger refused a confirmed deposit: {:?}", receipts[0]);
        };

        // And Alice's wallet, scanning what anyone could download, finds it.
        let enc = match &**op {
            Op::Deposit { enc, .. } => enc.clone(),
            _ => unreachable!(),
        };
        let wallet = Scanner::new(alice_view(), [0x31; 32]).scan(
            &ALICE,
            &[PublishedOutput { commitment, enc }],
            0,
            &[],
        );
        assert_eq!(wallet.live.len(), 1, "the holder must be able to find it");
        assert_eq!(wallet.balance(&ZEC), Fixed::whole(7).0);
    }

    /// The reason the nonce is derived. A bridge that dies after building an
    /// intent and before learning whether it landed must rebuild the *same*
    /// object, or it publishes one the holder's wallet will never find.
    #[test]
    fn a_restarted_bridge_rebuilds_the_identical_intent() {
        let vm = vm_with_alice_registered();
        let before = plan(&vm, SEED, ZEC, &[credit_of([0xDD; 32], 7)]);
        // Same seed, same txid, a fresh process that remembers nothing.
        let after = plan(&vm, SEED, ZEC, &[credit_of([0xDD; 32], 7)]);
        assert_eq!(before, after, "a restart must not change the object");
    }

    /// Unguessable, not merely unique. Everything in a deposit except the
    /// seed is public, so the nonce must depend on the seed.
    #[test]
    fn the_nonce_is_not_derivable_from_public_information() {
        let txid = [0xDD; 32];
        let mine = deposit_nonce(SEED, &txid);
        let theirs = deposit_nonce(b"a seed an attacker guessed", &txid);
        assert_ne!(mine, theirs, "the nonce must depend on the seed");
    }

    /// Two deposits to one account must not share an ephemeral key: a shared
    /// one links them to the same recipient for anyone reading the chain.
    #[test]
    fn two_deposits_do_not_share_an_ephemeral_key() {
        let a = deposit_ephemeral(SEED, &[0x01; 32]);
        let b = deposit_ephemeral(SEED, &[0x02; 32]);
        assert_ne!(a.to_bytes(), b.to_bytes());
        assert_ne!(
            deposit_nonce(SEED, &[0x01; 32]),
            deposit_nonce(SEED, &[0x02; 32])
        );
        // And the two derivations must not collide with each other.
        assert_ne!(deposit_nonce(SEED, &[0x01; 32]), a.to_bytes());
    }

    /// A deposit for someone who has not registered is money in the vault
    /// with nowhere to go yet. It must not be dropped, and it must not be
    /// credited to an object nobody can open.
    #[test]
    fn a_deposit_for_an_unregistered_account_waits_rather_than_vanishing() {
        let vm = ObjectVm::genesis(212, Limits { max_io: 8 });
        let planned = plan(&vm, SEED, ZEC, &[credit_of([0xDD; 32], 7)]);
        assert!(
            matches!(planned[0], Planned::AwaitingViewingKey { .. }),
            "got {:?}",
            planned[0]
        );

        // And it becomes creditable the moment they register — nothing about
        // the wait has expired it.
        let mut vm = vm;
        vm.apply(
            1,
            &Op::Register {
                account: ALICE,
                program: program::register_viewing_key(),
                nullifier_commitment: ObjectVm::nullifier_commitment(&NKEY),
                viewing: viewing_public(&alice_view()),
            },
        );
        let planned = plan(&vm, SEED, ZEC, &[credit_of([0xDD; 32], 7)]);
        assert!(matches!(planned[0], Planned::Submit(_)), "got {:?}", planned[0]);
    }

    /// Resubmitting after a restart is safe and says so: the deposit is
    /// reported as already credited rather than as an error to retry.
    #[test]
    fn a_replayed_deposit_is_recognised_not_retried() {
        let mut vm = vm_with_alice_registered();
        let planned = plan(&vm, SEED, ZEC, &[credit_of([0xDD; 32], 7)]);
        let Planned::Submit(op) = &planned[0] else { panic!() };
        vm.apply(2, op);

        let again = plan(&vm, SEED, ZEC, &[credit_of([0xDD; 32], 7)]);
        assert_eq!(again[0], Planned::AlreadyCredited { txid: [0xDD; 32] });
    }

    /// An attestation has no intent to become, so it must come back to the
    /// caller rather than being dropped on the floor.
    #[test]
    fn attestations_are_handed_back_not_discarded() {
        let vm = vm_with_alice_registered();
        let planned = plan(
            &vm,
            SEED,
            ZEC,
            &[
                WatcherAction::Attest {
                    observed: Fixed::whole(100),
                },
                credit_of([0xDD; 32], 7),
            ],
        );
        assert_eq!(
            planned[0],
            Planned::Attestation {
                asset: None,
                observed: Fixed::whole(100)
            }
        );
        assert!(matches!(planned[1], Planned::Submit(_)));
    }

    /// The solvency question an attestation actually answers.
    #[test]
    fn solvency_compares_what_the_vault_holds_against_what_was_issued() {
        let mut vm = vm_with_alice_registered();
        let planned = plan(&vm, SEED, ZEC, &[credit_of([0xDD; 32], 7)]);
        let Planned::Submit(op) = &planned[0] else { panic!() };
        vm.apply(2, op);

        assert!(solvent(&vm, &ZEC, Fixed::whole(7)), "exactly backed is solvent");
        assert!(solvent(&vm, &ZEC, Fixed::whole(9)), "over-backed is fine");
        assert!(
            !solvent(&vm, &ZEC, Fixed::whole(6)),
            "short by one is the case this exists to catch"
        );
    }

    /// The bridge's own asset arrives as `None` and must land on the
    /// configured id, not on a zero address.
    #[test]
    fn the_bridges_own_asset_is_named_by_configuration() {
        let vm = vm_with_alice_registered();
        let planned = plan(&vm, SEED, ZEC, &[credit_of([0xDD; 32], 7)]);
        match &planned[0] {
            Planned::Submit(op) => match &**op {
                Op::Deposit { asset, .. } => assert_eq!(*asset, ZEC),
                _ => panic!("not a deposit"),
            },
            other => panic!("got {other:?}"),
        }
    }

    /// The salts are a wire format: they decide every deposit commitment the
    /// bridge has ever published. Changing one orphans all of them.
    #[test]
    fn the_salts_are_pinned() {
        assert_eq!(NONCE_SALT, b"zyn.ingress.nonce.v1");
        assert_eq!(EPHEMERAL_SALT, b"zyn.ingress.ephemeral.v1");
        assert_eq!(NONCE_INFO, b"nonce");
        assert_eq!(EPHEMERAL_INFO, b"ephemeral");
    }
}

/// Deposits the vault holds and the ledger has not credited.
///
/// A deposit for an account that never registered a viewing key cannot become
/// an object: the object would be one nobody can open. Refusing is right, and
/// refusing is not enough, because of what the watcher does next — it marks
/// that height scanned and records the txid as credited, and **never reports
/// the deposit again**. The pending list is then the only record anywhere
/// that the vault owes that money. In memory, a restart loses it, and the
/// holder's ZEC sits in the vault with nothing on either chain saying whose
/// it is (A117).
///
/// So it is written down, with the same discipline as the watcher cursor it
/// sits beside: a magic, a version, and a decode that refuses trailing bytes
/// rather than guessing.
///
/// # Why the height is kept
///
/// Because the list can still be wrong. If it is lost or truncated while the
/// cursor survives, the cursor says "scanned" for a height whose deposit was
/// never credited, and no forward scan will ever look there again. The height
/// is what makes that recoverable: rewind the watcher to the earliest pending
/// deposit and the range is read again. A pending entry without its height is
/// a debt with no way back to the evidence.
pub mod pending {
    use super::*;
    use std::collections::BTreeMap;
    use zyn_vm::commit::Encoder;
    use zyn_vm::read::Decoder;

    /// v1 of the pending list. Kept separate from the watcher's own progress
    /// file: losing one must not mean rewriting the other, and the set that
    /// prevents double-credits should never share a write with the set that
    /// records debts.
    const MAGIC: &[u8; 8] = b"ZYNPEND1";

    /// One deposit in the vault with nowhere to go yet.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub struct Debt {
        pub account: AccountId,
        pub amount: Fixed,
        pub txid: [u8; 32],
        /// The Zcash height it was mined at. The way back to the evidence.
        pub height: u64,
    }

    /// Everything the vault owes and has not credited.
    ///
    /// Keyed by txid, so recording the same deposit twice is not a second
    /// debt — which matters because a rewind replays a range deliberately.
    #[derive(Clone, PartialEq, Eq, Debug, Default)]
    pub struct Pending {
        debts: BTreeMap<[u8; 32], Debt>,
    }

    impl Pending {
        pub fn new() -> Pending {
            Pending::default()
        }

        pub fn len(&self) -> usize {
            self.debts.len()
        }
        pub fn is_empty(&self) -> bool {
            self.debts.is_empty()
        }

        /// Everything owed, in a stable order. Oldest height first: it is the
        /// order these should be retried in and the order a reviewer checks.
        pub fn debts(&self) -> Vec<Debt> {
            let mut v: Vec<Debt> = self.debts.values().copied().collect();
            v.sort_by_key(|d| (d.height, d.txid));
            v
        }

        /// The earliest height still owed. What to rewind a watcher to when
        /// the cursor and this list disagree.
        pub fn earliest_height(&self) -> Option<u64> {
            self.debts.values().map(|d| d.height).min()
        }

        /// Take note of everything a pass could not credit.
        ///
        /// `height` is the height the pass covered. Recording a debt is the
        /// last thing that should ever be skipped, so this cannot fail.
        pub fn record(&mut self, planned: &[Planned], height: u64) {
            for p in planned {
                if let Planned::AwaitingViewingKey {
                    account,
                    amount,
                    txid,
                } = p
                {
                    self.debts.entry(*txid).or_insert(Debt {
                        account: *account,
                        amount: *amount,
                        txid: *txid,
                        height,
                    });
                }
            }
        }

        /// Forget a debt, once the ledger has actually taken it.
        ///
        /// Only call this against a receipt. Dropping a debt because an
        /// intent was *built* would lose it in exactly the window this
        /// module exists to cover.
        pub fn settle(&mut self, txid: &[u8; 32]) -> bool {
            self.debts.remove(txid).is_some()
        }

        /// The credits to retry, for every debt whose holder has since
        /// registered.
        ///
        /// Returns watcher actions rather than intents so the retry path and
        /// the live path are the same code — a retry that went down a
        /// different road would be a road nothing tests.
        pub fn replayable(&self, vm: &ObjectVm) -> Vec<WatcherAction> {
            self.debts()
                .into_iter()
                .filter(|d| vm.viewing.contains_key(&d.account))
                .map(|d| WatcherAction::Credit {
                    account: d.account,
                    amount: d.amount,
                    index: 0,
                    external_ref: d.txid,
                    asset: None,
                })
                .collect()
        }

        pub fn encode(&self) -> Vec<u8> {
            let mut e = Encoder::new();
            e.bytes(MAGIC).u32(self.debts.len() as u32);
            for d in self.debts() {
                e.bytes(&d.account)
                    .fixed(d.amount)
                    .bytes(&d.txid)
                    .u64(d.height);
            }
            e.finish().to_vec()
        }

        /// Refuses anything it does not fully understand.
        ///
        /// A half-read debts file is worse than none: it would report a
        /// shorter list of what the vault owes, and the missing entries are
        /// invisible rather than wrong. `None` tells the caller to recover
        /// from the chain instead of trusting a number.
        pub fn decode(b: &[u8]) -> Option<Pending> {
            let mut d = Decoder::new(b);
            if d.array::<8>().ok()? != *MAGIC {
                return None;
            }
            let n = d.u32().ok()?;
            let mut debts = BTreeMap::new();
            for _ in 0..n {
                let account = d.account().ok()?;
                let amount = d.fixed().ok()?;
                let txid = d.hash().ok()?;
                let height = d.u64().ok()?;
                debts.insert(
                    txid,
                    Debt {
                        account,
                        amount,
                        txid,
                        height,
                    },
                );
            }
            if d.remaining() != 0 {
                return None;
            }
            Some(Pending { debts })
        }
    }
}
