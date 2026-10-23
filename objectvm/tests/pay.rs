//! Zyn Pay (spec 2026-10-23-zyn-scheduled-payments-design.md): scheduled
//! payments funded at approval, one-off payments, refunds. Every intent goes
//! through `apply`, and after each one the books must still balance.

use objectvm::pay::{self, Every};
use objectvm::{Limits, ObjectVm, Op, Output, Receipt};
use zyn_vm::commit::Hash;
use zyn_vm::note_enc::{seal, viewing_public, StaticSecret};
use zyn_vm::object::{nullifier, Note};
use zyn_vm::spec::{AccountId, MicrochainVm};
use zyn_vm::{program, Fixed};

const BOLD: [u8; 32] = [0xB1; 32];
const ALICE: AccountId = [0xA1; 32];
const SHOP: AccountId = [0x5E; 32];
#[allow(dead_code)] // used by the schedule tests that follow
const DAY: u64 = 86_400;
// 2026-11-01 00:00:00 UTC
const NOV1: u64 = 1_793_491_200;

fn enc() -> zyn_vm::note_enc::EncryptedNote {
    seal(&viewing_public(&StaticSecret::from([0x11; 32])), StaticSecret::from([0x42; 32]), b"n").unwrap()
}

struct Chain {
    vm: ObjectVm,
    seq: u64,
    salt: u64,
    /// The pay clock, as far as this harness has moved it.
    clock: u64,
}

#[allow(dead_code)] // the pay behaviour tests that follow use the rest
impl Chain {
    fn new() -> Chain {
        let mut c = Chain { vm: ObjectVm::genesis(212, Limits { max_io: 16 }), seq: 0, salt: 0, clock: 0 };
        for who in [ALICE, SHOP] {
            c.ok(Op::Register { account: who, program: program::register_viewing_key(), viewing: viewing_public(&StaticSecret::from([who[0]; 32])) });
        }
        c
    }

    fn apply(&mut self, op: Op) -> Receipt {
        self.seq += 1;
        let r = self.vm.apply(self.seq, &op);
        assert_eq!(self.vm.conserved(), Ok(()), "after {op:?}");
        assert_eq!(self.vm.reconciled(), Ok(()));
        let back = ObjectVm::decode(&self.vm.encode()).expect("state decodes");
        assert_eq!(back, self.vm, "state round trip");
        let wire = ObjectVm::encode_intent(&op);
        let mut d = zyn_vm::read::Decoder::new(&wire);
        assert_eq!(ObjectVm::decode_intent(&mut d).as_ref(), Some(&op), "intent round trip");
        r.into_iter().next().unwrap()
    }

    fn ok(&mut self, op: Op) -> Receipt {
        let r = self.apply(op);
        assert!(!ObjectVm::rejected(&r), "{r:?}");
        r
    }

    fn refused(&mut self, op: Op) -> &'static str {
        match self.apply(op) {
            Receipt::Refused(why) => why,
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    fn fresh(&mut self) -> Hash {
        self.salt += 1;
        let mut h = [0x77u8; 32];
        h[..8].copy_from_slice(&self.salt.to_be_bytes());
        h
    }

    fn mint(&mut self, owner: AccountId, asset: [u8; 32], amount: Fixed) {
        let nonce = self.fresh();
        self.ok(Op::Mint { asset, amount, owner, nonce });
    }

    fn balance(&self, owner: AccountId, asset: [u8; 32]) -> i128 {
        self.vm.live.values().filter(|n| n.owner == owner && n.asset == asset).map(|n| n.amount.0).sum()
    }

    /// Inputs covering `amount` of the owner's notes, and the change note.
    fn fund(&mut self, owner: AccountId, asset: [u8; 32], amount: Fixed) -> (Vec<(Hash, Hash)>, Vec<Output>) {
        let mut picked = Vec::new();
        let mut sum = 0i128;
        let notes: Vec<(Hash, Note)> = self.vm.live.iter().map(|(c, n)| (*c, *n)).collect();
        for (c, n) in notes {
            if sum >= amount.0 {
                break;
            }
            if n.owner == owner && n.asset == asset {
                picked.push((c, nullifier(&[owner[0]; 32], &c)));
                sum += n.amount.0;
            }
        }
        assert!(sum >= amount.0, "too little to fund it");
        let change = if sum > amount.0 {
            let nonce = self.fresh();
            vec![Output { note: Note { asset, amount: Fixed(sum - amount.0), owner, nonce }, enc: enc() }]
        } else {
            Vec::new()
        };
        (picked, change)
    }

    fn claimable(&self, owner: AccountId, asset: [u8; 32]) -> i128 {
        self.vm.market.claims.values().filter(|c| c.owner == owner && c.asset == asset).map(|c| c.amount.0).sum()
    }

    /// Alice subscribes to SHOP: `amount` every `every`, `periods` total,
    /// `fund` periods set aside now. Returns the schedule id.
    fn subscribe(&mut self, amount: Fixed, every: Every, first_due: u64, periods: Option<u32>, fund: u32) -> Hash {
        let nonce = self.fresh();
        let total = Fixed(amount.0 * fund as i128);
        let (inputs, change) = self.fund(ALICE, BOLD, total);
        self.ok(Op::PayCreate {
            payer: ALICE,
            program: program::pay_create(),
            payee: SHOP,
            asset: BOLD,
            amount,
            every,
            first_due,
            periods,
            fund_periods: fund,
            reference: [0xEF; 32],
            nonce,
            inputs,
            change,
        });
        pay::schedule_id(&ALICE, &nonce)
    }

    fn clock(&mut self, at: u64) -> Receipt {
        self.apply(Op::PayClock { program: program::pay_clock(), at })
    }

    /// Move the clock to `to` in steps the clock rule allows.
    fn clock_to(&mut self, to: u64) {
        let mut at = self.clock;
        while at < to {
            at = if at == 0 { to } else { (at + pay::MAX_PAY_CLOCK_STEP_SECS).min(to) };
            assert!(!ObjectVm::rejected(&self.clock(at)));
            self.clock = at;
        }
    }
}

fn whole(n: i128) -> Fixed {
    Fixed(n * 1_000_000_000_000_000_000)
}

#[test]
fn pay_ops_round_trip_and_name_their_signer() {
    let ops = [
        Op::PayCreate { payer: ALICE, program: program::pay_create(), payee: SHOP, asset: BOLD, amount: whole(10), every: Every::Months(1), first_due: NOV1, periods: Some(12), fund_periods: 3, reference: [1; 32], nonce: [2; 32], inputs: vec![([3; 32], [4; 32])], change: Vec::new() },
        Op::PayTopUp { payer: ALICE, program: program::pay_manage(), schedule: [5; 32], periods: 2, inputs: vec![([3; 32], [4; 32])], change: Vec::new() },
        Op::PayCancel { payer: ALICE, program: program::pay_manage(), schedule: [5; 32] },
        Op::PayOnce { payer: ALICE, program: program::pay_once(), payee: SHOP, asset: BOLD, amount: whole(1), reference: [1; 32], nonce: [6; 32], inputs: vec![([3; 32], [4; 32])], change: Vec::new() },
        Op::PayRefund { payee: SHOP, program: program::pay_refund(), payment: [7; 32], amount: whole(1), inputs: vec![([3; 32], [4; 32])], change: Vec::new() },
        Op::PayClock { program: program::pay_clock(), at: NOV1 },
    ];
    let signers = [vec![ALICE], vec![ALICE], vec![ALICE], vec![ALICE], vec![SHOP], vec![]];
    for (op, want) in ops.iter().zip(signers) {
        let wire = ObjectVm::encode_intent(op);
        let mut d = zyn_vm::read::Decoder::new(&wire);
        assert_eq!(ObjectVm::decode_intent(&mut d).as_ref(), Some(op));
        assert_eq!(ObjectVm::intent_authorities(op), want, "{op:?}");
    }
}
