//! SOL.zy and ETH.zy on the object ledger (DECISIONS §146): an origin
//! registered from public facts, deposits that name no account, claims only
//! the committed account can make, and the ordinary way out — with the books
//! checked after every intent.

use objectvm::bridge::{asset_of, deposit_commitment, deposit_key, valid_origin};
use objectvm::{Limits, ObjectVm, Op, Output, Receipt};
use zyn_vm::commit::Hash;
use zyn_vm::note_enc::{seal, viewing_public, StaticSecret};
use zyn_vm::object::{commitment, nullifier, Note};
use zyn_vm::spec::{AccountId, MicrochainVm};
use zyn_vm::{program, Fixed};

const ALICE: AccountId = [0xA1; 32];
const BOB: AccountId = [0xB0; 32];

struct Chain {
    vm: ObjectVm,
    seq: u64,
}

fn enc() -> zyn_vm::note_enc::EncryptedNote {
    seal(&viewing_public(&StaticSecret::from([0x11; 32])), StaticSecret::from([0x42; 32]), b"n").unwrap()
}

fn out(owner: AccountId, asset: [u8; 32], amount: Fixed, nonce: u8) -> Output {
    Output { note: Note { asset, amount, owner, nonce: [nonce; 32] }, enc: enc() }
}

impl Chain {
    fn apply(&mut self, op: Op) -> Receipt {
        self.seq += 1;
        let r = self.vm.apply(self.seq, &op);
        assert_eq!(self.vm.conserved(), Ok(()), "after {op:?}");
        assert_eq!(self.vm.reconciled(), Ok(()));
        assert_eq!(ObjectVm::decode(&self.vm.encode()).as_ref(), Some(&self.vm), "state round-trips");
        let wire = ObjectVm::encode_intent(&op);
        assert_eq!(ObjectVm::decode_intent(&mut zyn_vm::read::Decoder::new(&wire)).as_ref(), Some(&op), "intent round-trips");
        r.into_iter().next().unwrap()
    }
    fn ok(&mut self, op: Op) -> Receipt {
        let r = self.apply(op);
        assert!(!ObjectVm::rejected(&r), "{r:?}");
        r
    }
    fn refused(&mut self, op: Op) -> String {
        match self.apply(op) {
            Receipt::Refused(why) => why.to_string(),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }
}

fn register(network: &[u8], decimals: u8) -> Op {
    Op::RegisterBridged { program: program::bridge_register(), network: network.to_vec(), origin: Vec::new(), decimals }
}

fn deposit(asset: [u8; 32], whole: i64, reference: u8, commitment: Hash) -> Op {
    Op::DepositCommitted { program: program::deposit(), asset, gross: Fixed::whole(whole), external_ref: [reference; 32], commitment }
}

#[test]
fn the_origin_registry_is_the_frozen_one() {
    assert!(valid_origin(b"solana-devnet", b""));
    assert!(valid_origin(b"eip155:11155111", b""));
    assert!(valid_origin(b"eip155:1", &[7; 20]));
    assert!(!valid_origin(b"ethereum", b""), "a family is not a network");
    assert!(!valid_origin(b"eip155:01", b""), "no leading zeros: one chain, one name");
    assert!(!valid_origin(b"solana", &[1; 20]), "a Solana mint is 32 bytes");
    assert_ne!(asset_of(b"solana", b""), asset_of(b"solana-devnet", b""), "devnet SOL is not SOL");
}

#[test]
fn sol_in_by_commitment_claimed_only_by_its_account_and_out_again() {
    let mut c = Chain { vm: ObjectVm::genesis(212, Limits { max_io: 16 }), seq: 0 };
    let sol = asset_of(b"solana-devnet", b"");
    let reg = register(b"solana-devnet", 9);
    assert_eq!(ObjectVm::intent_authorities(&reg), Vec::<AccountId>::new(), "only the operator registers");
    assert_eq!(c.ok(reg), Receipt::BridgedRegistered { asset: sol });
    assert!(c.refused(register(b"solana-devnet", 9)).contains("already registered"));
    assert!(c.refused(register(b"ethereum", 18)).contains("registry"));

    let secret = [0x5e; 32];
    let com = deposit_commitment(&ALICE, &secret);
    let unregistered = asset_of(b"eip155:11155111", b"");
    assert!(c.refused(deposit(unregistered, 1, 1, com)).contains("not bridged"));

    // Two deposits on one commitment wait together; one reference counts once.
    let d = deposit(sol, 2, 1, com);
    assert_eq!(ObjectVm::intent_authorities(&d), Vec::<AccountId>::new(), "only the operator records a deposit");
    c.ok(d);
    assert!(c.refused(deposit(sol, 2, 1, com)).contains("already credited"));
    c.ok(deposit(sol, 3, 2, com));
    assert_eq!(c.vm.market.waiting[&deposit_key(&com, &sol)].amount, Fixed::whole(5));
    assert_eq!(c.vm.issued[&sol], Fixed::whole(5), "issued when the vault holds it");

    // Bob knows the secret (say Alice's claim leaked it): still Alice's.
    let claim = |owner: AccountId, amount: Fixed, n: u8| Op::ClaimDeposit { owner, program: program::claim(), secret, asset: sol, output: out(owner, sol, amount, n) };
    assert!(c.refused(claim(BOB, Fixed::whole(5), 1)).contains("nothing is waiting"));
    assert!(c.refused(claim(ALICE, Fixed::whole(4), 2)).contains("everything waiting"));
    let take = claim(ALICE, Fixed::whole(5), 3);
    assert_eq!(ObjectVm::intent_authorities(&take), vec![ALICE]);
    assert!(matches!(c.ok(take), Receipt::Deposited { .. }));
    assert!(c.vm.market.waiting.is_empty());
    assert!(c.refused(claim(ALICE, Fixed::whole(5), 4)).contains("nothing is waiting"), "claimed once");

    // And out, the ordinary way: supply falls, an exit waits for Solana's payer.
    let note = Note { asset: sol, amount: Fixed::whole(5), owner: ALICE, nonce: [3; 32] };
    let cm = commitment(&note).unwrap();
    c.ok(Op::Withdraw {
        holder: ALICE,
        program: program::withdraw(),
        inputs: vec![(cm, nullifier(&[0xA1; 32], &cm))],
        asset: sol,
        amount: Fixed::whole(4),
        destination: [0xDD; 32],
        outputs: vec![out(ALICE, sol, Fixed::whole(1), 9)],
    });
    assert_eq!(c.vm.issued[&sol], Fixed::whole(1));
    assert_eq!(c.vm.exits.last().map(|e| (e.asset, e.amount)), Some((sol, Fixed::whole(4))));
}
