//! One pass of the deposit bridge, decided before anything is applied.
//!
//! [`object_ingress::plan`] turns watcher actions into intents, and the node
//! applies them. Between those two sits the question that decides whether the
//! ledger may issue at all: **does the vault still hold at least what the
//! ledger has already created?**
//!
//! # Why the watcher's attestation does not answer it
//!
//! The watcher emits `Attest { observed }` before the credits it covers, and
//! it is tempting to compare that against what the ledger has issued. It is
//! wrong here. `zebra::Observed::balance_at` sums *the deposits in the
//! snapshot it was built from*, and `Scanner::observe` builds that snapshot
//! from one scan range — so `observed` is the deposits in this range, not the
//! vault's balance. Compared against a cumulative `issued` it reads as a
//! shortfall the moment a second deposit arrives in a later range, and the
//! gate below would refuse every pass from then on.
//!
//! Its own doc says as much: the sum equals the balance "only while the vault
//! never spends", and this vault spends — every anchor is a self-send that
//! burns a fee.
//!
//! So the balance is taken from the note stores, which hold the vault's
//! unspent notes and therefore account for both deposits and the anchors that
//! spent them. The attestation is still checked, as a floor: it cannot prove
//! solvency, but a range whose own deposits exceed what the vault holds is
//! incoherent and worth refusing.
//!
//! This module is that gate, and it is separate from the node so it can be
//! tested without a Zcash node, a socket or a disk. The node does the I/O; the
//! decision lives here.
//!
//! # Why a pass is all-or-nothing
//!
//! A pass that credited the deposits it liked and stopped at the one it did
//! not would leave the ledger holding objects whose backing was never checked.
//! So an insolvent attestation refuses the **whole** pass, including credits
//! that would individually have been fine. The vault is not short by one
//! deposit; it is short, and nothing in this file can repair that.

use objectvm::ObjectVm;
use zyn_custody::watcher::{AssetId, WatcherAction};
use zyn_vm::Fixed;

use crate::object_ingress::{plan, solvent, Planned};

/// What a pass decided.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Pass {
    /// The vault holds less than the ledger has issued. Nothing is applied.
    ///
    /// This is an alarm, not a retry: no intent repairs it, and continuing to
    /// credit would deepen a hole rather than fill it.
    Insolvent {
        asset: Option<AssetId>,
        observed: Fixed,
        issued: Fixed,
    },
    /// An asset arrived whose backing this bridge cannot check. Nothing is
    /// applied: issuing against backing nobody verified is exactly what the
    /// gate exists to stop.
    Unverifiable { asset: AssetId },
    /// Backing checked. Apply these, in this order.
    Apply(Vec<Decided>),
}

/// One thing the pass decided to do, in the order it must happen.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Decided {
    /// Credit a confirmed deposit. `txid` is carried alongside so the caller
    /// can settle the matching debt against a *receipt* rather than against
    /// having built the intent.
    Credit {
        op: Box<objectvm::Op>,
        txid: [u8; 32],
    },
    /// The vault holds this and has nobody to give it to yet.
    Owed {
        account: zyn_vm::spec::AccountId,
        amount: Fixed,
        txid: [u8; 32],
    },
    /// Already on the ledger. Counted, not applied — the normal result of a
    /// restart, and the proof that the derived nonce did its job.
    AlreadyCredited { txid: [u8; 32] },
    /// The watcher reported something that cannot be credited without minting.
    Rejected {
        txid: [u8; 32],
        why: &'static str,
    },
}

/// Decide one pass: check the backing, then say what to apply.
///
/// `actions` is what the watcher produced, plus any retries for debts whose
/// holders have since registered. Both go through the same road deliberately —
/// a retry down a different path would be a path nothing tests.
pub fn decide(
    vm: &ObjectVm,
    seed: &[u8],
    zec: AssetId,
    actions: &[WatcherAction],
    vault: Fixed,
) -> Pass {
    let planned = plan(vm, seed, zec, actions);

    // The gate. `vault` is what the vault actually holds, unspent, across
    // every pool — the note stores' own figure, not the range sum the
    // watcher attests to (see the module doc).
    if !solvent(vm, &zec, vault) {
        return Pass::Insolvent {
            asset: None,
            observed: vault,
            issued: vm.issued.get(&zec).copied().unwrap_or(Fixed::ZERO),
        };
    }

    for p in &planned {
        match p {
            // A range that says it received more than the vault holds is
            // incoherent: those deposits are inside that balance.
            Planned::Attestation { asset: None, observed } if observed.0 > vault.0 => {
                return Pass::Insolvent {
                    asset: None,
                    observed: *observed,
                    issued: vm.issued.get(&zec).copied().unwrap_or(Fixed::ZERO),
                };
            }
            // An asset this bridge does not custody. Its backing cannot be
            // checked from here, and issuing against backing nothing verified
            // is the one thing this gate exists to prevent, so refuse rather
            // than wave it through.
            Planned::Attestation { asset: Some(a), .. } => {
                return Pass::Unverifiable { asset: *a };
            }
            _ => {}
        }
    }

    Pass::Apply(
        planned
            .into_iter()
            .filter_map(|p| match p {
                Planned::Submit(op) => {
                    let txid = external_ref_of(&op)?;
                    Some(Decided::Credit { op, txid })
                }
                Planned::AwaitingViewingKey {
                    account,
                    amount,
                    txid,
                } => Some(Decided::Owed {
                    account,
                    amount,
                    txid,
                }),
                Planned::AlreadyCredited { txid } => Some(Decided::AlreadyCredited { txid }),
                Planned::Rejected { txid, why } => Some(Decided::Rejected { txid, why }),
                // Attestations were the gate above and carry nothing to apply.
                Planned::Attestation { .. } => None,
            })
            .collect(),
    )
}

/// The Zcash transaction a credit is for.
///
/// `ingress::credit` only ever builds a `Deposit`, but reading the field back
/// rather than assuming it keeps the debt bookkeeping honest if that changes.
fn external_ref_of(op: &objectvm::Op) -> Option<[u8; 32]> {
    match op {
        objectvm::Op::Deposit { external_ref, .. }
        | objectvm::Op::DepositWithFee { external_ref, .. } => Some(*external_ref),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use objectvm::{Limits, Op, Receipt};
    use zyn_vm::note_enc::{viewing_public, StaticSecret};
    use zyn_vm::program;
    use zyn_vm::spec::{AccountId, MicrochainVm};

    const SEED: &[u8] = b"a bridge seed";
    const ZEC: AssetId = [0x7a; 32];
    const ALICE: AccountId = [0xA1; 32];

    fn vm_with_alice_registered() -> ObjectVm {
        let mut vm = ObjectVm::genesis(212, Limits { max_io: 8 });
        let r = vm.apply(
            1,
            &Op::Register {
                account: ALICE,
                program: program::register_viewing_key(),
                viewing: viewing_public(&StaticSecret::from([0xA1; 32])),
            },
        );
        assert!(matches!(r[0], Receipt::Registered { .. }));
        vm
    }

    fn credit(amount: i64, txid: u8) -> WatcherAction {
        WatcherAction::Credit {
            account: ALICE,
            amount: Fixed::whole(amount),
            index: 0,
            external_ref: [txid; 32],
            asset: None,
        }
    }

    /// The ordinary path: the vault attests to holding enough, and the
    /// deposit becomes an object.
    #[test]
    fn a_backed_deposit_is_credited() {
        let vm = vm_with_alice_registered();
        let actions = vec![
            WatcherAction::Attest {
                observed: Fixed::whole(5),
            },
            credit(5, 0x11),
        ];
        match decide(&vm, SEED, ZEC, &actions, Fixed::whole(5)) {
            Pass::Apply(d) => {
                assert_eq!(d.len(), 1);
                assert!(matches!(&d[0], Decided::Credit { txid, .. } if *txid == [0x11; 32]));
            }
            other => panic!("expected a credit: {other:?}"),
        }
    }

    /// The gate. If the vault holds less than the ledger has issued, objects
    /// exist that nothing backs, and no intent repairs that.
    #[test]
    fn a_vault_holding_less_than_was_issued_refuses_the_whole_pass() {
        let mut vm = vm_with_alice_registered();
        // Issue 10 against the ledger.
        let op = objectvm::ingress::credit(
            &vm,
            ZEC,
            Fixed::whole(10),
            ALICE,
            [0x01; 32],
            [0x02; 32],
            StaticSecret::from([0x03; 32]),
        )
        .expect("alice is registered");
        vm.apply(2, &op);
        assert_eq!(vm.issued.get(&ZEC).copied().unwrap(), Fixed::whole(10));

        // The vault now attests to holding only 4.
        let actions = vec![
            WatcherAction::Attest {
                observed: Fixed::whole(4),
            },
            credit(1, 0x22),
        ];
        match decide(&vm, SEED, ZEC, &actions, Fixed::whole(4)) {
            Pass::Insolvent {
                observed, issued, ..
            } => {
                assert_eq!(observed, Fixed::whole(4));
                assert_eq!(issued, Fixed::whole(10));
            }
            other => panic!("an under-backed vault must refuse the pass: {other:?}"),
        }
    }

    /// And the refusal takes the *whole* pass with it — including a credit
    /// that would have been fine on its own. The vault is not short by one
    /// deposit; it is short.
    #[test]
    fn insolvency_refuses_even_the_credits_that_would_have_been_fine() {
        let mut vm = vm_with_alice_registered();
        let op = objectvm::ingress::credit(
            &vm,
            ZEC,
            Fixed::whole(10),
            ALICE,
            [0x01; 32],
            [0x02; 32],
            StaticSecret::from([0x03; 32]),
        )
        .unwrap();
        vm.apply(2, &op);
        let actions = vec![
            credit(1, 0x33), // would have been fine
            WatcherAction::Attest {
                observed: Fixed::whole(1),
            },
            credit(1, 0x44),
        ];
        assert!(matches!(
            decide(&vm, SEED, ZEC, &actions, Fixed::whole(1)),
            Pass::Insolvent { .. }
        ));
    }

    /// The bug this gate was written with, and the reason it does not read
    /// the watcher's attestation.
    ///
    /// `Scanner::observe` builds its snapshot from one scan range, so the
    /// attested figure is *that range's* deposits — not the vault's balance.
    /// A ledger that has issued 10 and then sees a fresh 1 ZEC deposit in a
    /// later range gets `observed = 1`, which against a cumulative `issued`
    /// of 10 reads as a vault that has lost 9. It has lost nothing. Gating on
    /// that number would have halted the bridge on the second deposit and
    /// called it an emergency.
    #[test]
    fn a_later_range_holding_one_deposit_is_not_an_insolvent_vault() {
        let mut vm = vm_with_alice_registered();
        let op = objectvm::ingress::credit(
            &vm,
            ZEC,
            Fixed::whole(10),
            ALICE,
            [0x01; 32],
            [0x02; 32],
            StaticSecret::from([0x03; 32]),
        )
        .unwrap();
        vm.apply(2, &op);

        // A later range carrying one new deposit. The attestation covers only
        // that range; the vault actually holds 11.
        let actions = vec![
            WatcherAction::Attest {
                observed: Fixed::whole(1),
            },
            credit(1, 0x77),
        ];
        match decide(&vm, SEED, ZEC, &actions, Fixed::whole(11)) {
            Pass::Apply(d) => {
                assert!(
                    d.iter().any(|x| matches!(x, Decided::Credit { .. })),
                    "the deposit must still be credited: {d:?}"
                );
            }
            other => panic!("a healthy vault must not read as insolvent: {other:?}"),
        }
    }

    /// The attestation is still a floor. A range claiming to have received
    /// more than the vault holds is incoherent — those deposits are inside
    /// that balance — and is refused.
    #[test]
    fn a_range_claiming_more_than_the_vault_holds_is_refused() {
        let vm = vm_with_alice_registered();
        let actions = vec![
            WatcherAction::Attest {
                observed: Fixed::whole(50),
            },
            credit(50, 0x88),
        ];
        assert!(matches!(
            decide(&vm, SEED, ZEC, &actions, Fixed::whole(2)),
            Pass::Insolvent { .. }
        ));
    }

    /// An asset this bridge does not custody cannot have its backing checked
    /// here, so the pass is refused rather than waved through.
    #[test]
    fn an_asset_the_bridge_does_not_custody_is_refused_not_assumed() {
        let vm = vm_with_alice_registered();
        let actions = vec![WatcherAction::AttestAsset {
            asset: [0xEE; 32],
            observed: Fixed::whole(1),
        }];
        assert_eq!(
            decide(&vm, SEED, ZEC, &actions, Fixed::whole(1_000)),
            Pass::Unverifiable { asset: [0xEE; 32] }
        );
    }

    /// Holding *more* than was issued is normal — a top-up credits nobody.
    #[test]
    fn a_vault_holding_more_than_was_issued_is_fine() {
        let vm = vm_with_alice_registered();
        let actions = vec![WatcherAction::Attest {
            observed: Fixed::whole(1_000),
        }];
        assert_eq!(
            decide(&vm, SEED, ZEC, &actions, Fixed::whole(1_000)),
            Pass::Apply(Vec::new())
        );
    }

    /// A deposit for somebody who has never said where to pay them is owed,
    /// not dropped. The ZEC stays in the vault and nothing expires it.
    #[test]
    fn a_deposit_for_an_unregistered_holder_is_owed() {
        let vm = ObjectVm::genesis(212, Limits { max_io: 8 }); // nobody registered
        let actions = vec![
            WatcherAction::Attest {
                observed: Fixed::whole(5),
            },
            credit(5, 0x55),
        ];
        match decide(&vm, SEED, ZEC, &actions, Fixed::whole(5)) {
            Pass::Apply(d) => {
                assert!(matches!(&d[0], Decided::Owed { txid, .. } if *txid == [0x55; 32]));
            }
            other => panic!("expected a debt: {other:?}"),
        }
    }

    /// A restart rebuilds the identical intent, so the ledger refuses it as a
    /// duplicate — which is proof the work was done, not a loss.
    #[test]
    fn a_deposit_already_on_the_ledger_is_reported_as_such() {
        let mut vm = vm_with_alice_registered();
        let actions = vec![
            WatcherAction::Attest {
                observed: Fixed::whole(7),
            },
            credit(7, 0x66),
        ];
        let Pass::Apply(first) = decide(&vm, SEED, ZEC, &actions, Fixed::whole(7)) else {
            panic!("expected a credit")
        };
        let Decided::Credit { op, .. } = &first[0] else {
            panic!("expected a credit")
        };
        vm.apply(2, op);

        // The same pass again, as a restart would produce it.
        match decide(&vm, SEED, ZEC, &actions, Fixed::whole(7)) {
            Pass::Apply(d) => {
                assert!(matches!(&d[0], Decided::AlreadyCredited { txid } if *txid == [0x66; 32]));
            }
            other => panic!("expected AlreadyCredited: {other:?}"),
        }
    }
}
