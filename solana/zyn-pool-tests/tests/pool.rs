//! The pool program as deployed: the SBF build of `solana/zyn-pool`, run in
//! LiteSVM, spent with real proofs from `zyn-pool` under the testnet keys it
//! embeds. Build the program first: `cargo build-sbf --tools-version v1.52`
//! in `solana/zyn-pool`.

use ark_bn254::Fr;
use base64::Engine;
use litesvm::LiteSVM;
use solana_address::Address as Pubkey;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::{read_keypair_file, Keypair};
use solana_signer::Signer;
use solana_system_interface::program as system_program;
use solana_transaction::Transaction;
use zyn_pool::circuit::Spend;
use zyn_pool::export::ProofBytes;
use zyn_pool::tree::Tree;
use zyn_pool::{binding_to_address, binding_to_zyn, fr_be, keys, Note};
// The program's addresses and instruction tags, restated: the program is
// built against Solana 2.x crates and these tests against 3.x, so they share
// bytes, not types.
const IX_INITIALIZE: u8 = 0;
const IX_DEPOSIT: u8 = 1;
const IX_SPEND_TO_ZYN: u8 = 2;
const IX_SPEND_TO_ADDRESS: u8 = 3;
const IX_INSERT_EXIT: u8 = 4;

fn pool_address(program: &Pubkey, denomination: u64) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"pool", &denomination.to_le_bytes()], program)
}

fn nullifier_address(program: &Pubkey, pool: &Pubkey, nullifier_hash: &[u8; 32]) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"nullifier", pool.as_ref(), nullifier_hash], program)
}

const SOL: u64 = 1_000_000_000;
const DENOM: u64 = SOL; // the 1 SOL pool

struct Pool {
    svm: LiteSVM,
    program: Pubkey,
    pool: Pubkey,
    payer: Keypair,
    authority: Keypair,
    tree: Tree,
    pk: ark_groth16::ProvingKey<ark_bn254::Bn254>,
}

fn program_id() -> Pubkey {
    read_keypair_file("../zyn-pool/target/deploy/zyn_pool_program-keypair.json").expect("build the program first").pubkey()
}

impl Pool {
    fn new() -> Pool {
        let program = program_id();
        let mut svm = LiteSVM::new();
        svm.add_program_from_file(program, "../zyn-pool/target/deploy/zyn_pool_program.so").expect("the .so");
        let payer = Keypair::new();
        let authority = Keypair::new();
        svm.airdrop(&payer.pubkey(), 100 * SOL).unwrap();
        svm.airdrop(&authority.pubkey(), SOL).unwrap();
        let (pool, _) = pool_address(&program, DENOM);
        let mut data = vec![IX_INITIALIZE];
        data.extend_from_slice(&DENOM.to_le_bytes());
        data.extend_from_slice(authority.pubkey().as_ref());
        let ix = Instruction::new_with_bytes(
            program,
            &data,
            vec![AccountMeta::new(payer.pubkey(), true), AccountMeta::new(pool, false), AccountMeta::new_readonly(system_program::id(), false)],
        );
        let (pk, _) = keys::setup(keys::testnet_seed());
        let mut p = Pool { svm, program, pool, payer, authority, tree: Tree::default(), pk };
        let payer = p.payer.insecure_clone();
        p.send(&[ix], &[&payer]).expect("initialize");
        p
    }

    fn send(&mut self, ixs: &[Instruction], signers: &[&Keypair]) -> Result<Vec<String>, String> {
        let mut all = vec![ComputeBudgetInstruction::set_compute_unit_limit(600_000)];
        all.extend_from_slice(ixs);
        self.svm.expire_blockhash();
        let tx = Transaction::new_signed_with_payer(&all, Some(&signers[0].pubkey()), signers, self.svm.latest_blockhash());
        self.svm.send_transaction(tx).map(|m| m.logs).map_err(|e| format!("{:?}: {}", e.err, e.meta.logs.join(" | ")))
    }

    fn balance(&self, who: &Pubkey) -> u64 {
        self.svm.get_account(who).map(|a| a.lamports).unwrap_or(0)
    }

    /// The fields of the program's `sol_log_data` event named `name`.
    fn event(logs: &[String], name: &[u8]) -> Vec<Vec<u8>> {
        let b64 = base64::engine::general_purpose::STANDARD;
        logs.iter()
            .filter_map(|l| l.strip_prefix("Program data: "))
            .map(|l| l.split(' ').map(|f| b64.decode(f).unwrap()).collect::<Vec<_>>())
            .find(|f| f[0] == name)
            .expect("the event")
    }

    fn deposit(&mut self, who: &Keypair, note: &Note) -> Result<usize, String> {
        let c = fr_be(note.commitment());
        let ix = Instruction::new_with_bytes(
            self.program,
            &[&[IX_DEPOSIT][..], &c].concat(),
            vec![AccountMeta::new(who.pubkey(), true), AccountMeta::new(self.pool, false), AccountMeta::new_readonly(system_program::id(), false)],
        );
        let logs = self.send(&[ix], &[who])?;
        let at = self.tree.insert(note.commitment());
        let ev = Pool::event(&logs, b"deposit");
        assert_eq!(ev[3], fr_be(self.tree.root()), "the on-chain root is the Rust tree's root");
        Ok(at)
    }

    fn proof(&self, note: &Note, at: usize, binding: Fr) -> (Vec<u8>, [u8; 32], [u8; 32]) {
        let s = Spend { root: self.tree.root(), nullifier_hash: note.nullifier_hash(), binding, nullifier: note.nullifier, secret: note.secret, path: self.tree.path(at) };
        let proof = keys::prove(&self.pk, s.clone(), [at as u8; 32]).unwrap();
        (ProofBytes::of(&proof).to_bytes().to_vec(), fr_be(s.root), fr_be(s.nullifier_hash))
    }

    fn spend_to_zyn(&mut self, proof: &[u8], root: [u8; 32], nh: [u8; 32], code: [u8; 32]) -> Result<Vec<String>, String> {
        let (nullifier, _) = nullifier_address(&self.program, &self.pool, &nh);
        let ix = Instruction::new_with_bytes(
            self.program,
            &[&[IX_SPEND_TO_ZYN][..], proof, &root, &nh, &code].concat(),
            vec![
                AccountMeta::new(self.payer.pubkey(), true),
                AccountMeta::new(self.pool, false),
                AccountMeta::new(nullifier, false),
                AccountMeta::new_readonly(system_program::id(), false),
            ],
        );
        let payer = self.payer.insecure_clone();
        self.send(&[ix], &[&payer])
    }

    fn spend_to_address(&mut self, relayer: &Keypair, recipient: &Pubkey, proof: &[u8], root: [u8; 32], nh: [u8; 32], fee: u64) -> Result<Vec<String>, String> {
        let (nullifier, _) = nullifier_address(&self.program, &self.pool, &nh);
        let ix = Instruction::new_with_bytes(
            self.program,
            &[&[IX_SPEND_TO_ADDRESS][..], proof, &root, &nh, &fee.to_le_bytes()].concat(),
            vec![
                AccountMeta::new(relayer.pubkey(), true),
                AccountMeta::new(self.pool, false),
                AccountMeta::new(nullifier, false),
                AccountMeta::new(*recipient, false),
                AccountMeta::new_readonly(system_program::id(), false),
            ],
        );
        self.send(&[ix], &[relayer])
    }

    fn insert_exit(&mut self, signer: &Keypair, note: &Note) -> Result<usize, String> {
        let c = fr_be(note.commitment());
        let ix = Instruction::new_with_bytes(
            self.program,
            &[&[IX_INSERT_EXIT][..], &c].concat(),
            vec![AccountMeta::new_readonly(signer.pubkey(), true), AccountMeta::new(self.pool, false)],
        );
        let payer = self.payer.insecure_clone();
        self.send(&[ix], &[&payer, signer])?;
        Ok(self.tree.insert(note.commitment()))
    }
}

fn note(n: u8) -> Note {
    Note::from_seed(&[n; 64])
}

#[test]
fn deposit_then_spend_into_zyn_once() {
    let mut p = Pool::new();
    let alice = Keypair::new();
    p.svm.airdrop(&alice.pubkey(), 5 * SOL).unwrap();
    let mine = note(1);
    for n in [10, 11] {
        p.deposit(&alice, &note(n)).unwrap();
    }
    let at = p.deposit(&alice, &mine).unwrap();
    p.deposit(&alice, &note(12)).unwrap();
    assert!(p.balance(&p.pool) >= 4 * DENOM, "the pool holds the four notes");

    let code = [0x42u8; 32];
    let (proof, root, nh) = p.proof(&mine, at, binding_to_zyn(&code));
    let logs = p.spend_to_zyn(&proof, root, nh, code).expect("the spend into Zyn");
    let ev = Pool::event(&logs, b"to_zyn");
    assert_eq!((ev[1].as_slice(), ev[2].as_slice()), (&nh[..], &code[..]), "the bridge learns the nullifier hash and the deposit code, not the note");
    let used: u64 = logs.iter().find_map(|l| l.split("consumed ").nth(1).and_then(|r| r.split(' ').next()).and_then(|n| n.parse().ok())).unwrap_or(0);
    println!("spend to Zyn: {used} compute units");
    assert!(p.balance(&p.pool) >= 4 * DENOM, "the SOL stays in the pool: it now backs SOL.zy");

    // The same note again: refused, whatever it is bound to.
    let (proof2, root2, nh2) = p.proof(&mine, at, binding_to_zyn(&[0x43u8; 32]));
    let err = p.spend_to_zyn(&proof2, root2, nh2, [0x43u8; 32]).unwrap_err();
    assert!(err.contains("Custom(4)"), "already spent: {err}");
}

#[test]
fn a_proof_cannot_be_pointed_somewhere_else() {
    let mut p = Pool::new();
    let alice = Keypair::new();
    p.svm.airdrop(&alice.pubkey(), 3 * SOL).unwrap();
    let mine = note(2);
    let at = p.deposit(&alice, &mine).unwrap();
    let (proof, root, nh) = p.proof(&mine, at, binding_to_zyn(&[1u8; 32]));
    let err = p.spend_to_zyn(&proof, root, nh, [2u8; 32]).unwrap_err();
    assert!(err.contains("Custom(3)"), "another deposit code: {err}");
    let err = p.spend_to_zyn(&proof, fr_be(zyn_pool::poseidon(&[Fr::from(7u64)])), nh, [1u8; 32]).unwrap_err();
    assert!(err.contains("Custom(2)"), "a root the pool never had: {err}");
}

#[test]
fn spend_to_a_fresh_address_through_a_relayer() {
    let mut p = Pool::new();
    let alice = Keypair::new();
    p.svm.airdrop(&alice.pubkey(), 3 * SOL).unwrap();
    let mine = note(3);
    let at = p.deposit(&alice, &mine).unwrap();
    p.deposit(&alice, &note(13)).unwrap();

    let relayer = Keypair::new();
    p.svm.airdrop(&relayer.pubkey(), SOL).unwrap();
    let fresh = Pubkey::new_unique();
    let fee = 2_000_000; // 0.002 SOL
    let (proof, root, nh) = p.proof(&mine, at, binding_to_address(fresh.as_ref(), relayer.pubkey().as_ref(), fee));

    // Someone watching the relayer's transaction swaps in their own address: no.
    let thief = Pubkey::new_unique();
    let err = p.spend_to_address(&relayer, &thief, &proof, root, nh, fee).unwrap_err();
    assert!(err.contains("Custom(3)"), "another recipient: {err}");
    // Nor can the relayer raise its own fee.
    let err = p.spend_to_address(&relayer, &fresh, &proof, root, nh, fee * 2).unwrap_err();
    assert!(err.contains("Custom(3)"), "another fee: {err}");

    let relayer_before = p.balance(&relayer.pubkey());
    p.spend_to_address(&relayer, &fresh, &proof, root, nh, fee).expect("the honest spend");
    assert_eq!(p.balance(&fresh), DENOM - fee, "a never-used address receives the note less the fee");
    assert!(p.balance(&relayer.pubkey()) + 5_000_000 > relayer_before + fee - 5_000_000, "the relayer is paid its fee (less what it spent on the transaction and the nullifier)");
}

#[test]
fn exits_come_from_the_custodians_and_never_outrun_the_pool() {
    let mut p = Pool::new();
    let alice = Keypair::new();
    p.svm.airdrop(&alice.pubkey(), 3 * SOL).unwrap();
    // One private deposit spent into Zyn: its SOL stays and backs one exit.
    let d = note(4);
    let at = p.deposit(&alice, &d).unwrap();
    let (proof, root, nh) = p.proof(&d, at, binding_to_zyn(&[9u8; 32]));
    p.spend_to_zyn(&proof, root, nh, [9u8; 32]).unwrap();

    let stranger = Keypair::new();
    p.svm.airdrop(&stranger.pubkey(), SOL).unwrap();
    let err = p.insert_exit(&stranger, &note(20)).unwrap_err();
    assert!(err.contains("Custom(6)"), "only the custodians' key: {err}");

    let authority = p.authority.insecure_clone();
    let exit = note(21);
    let at = p.insert_exit(&authority, &exit).expect("an exit the pool can pay");
    let err = p.insert_exit(&authority, &note(22)).unwrap_err();
    assert!(err.contains("Custom(7)"), "a second exit with nothing behind it: {err}");

    // The holder of the exit note takes it to an address of their choice.
    let fresh = Pubkey::new_unique();
    let relayer = p.payer.insecure_clone();
    let (proof, root, nh) = p.proof(&exit, at, binding_to_address(fresh.as_ref(), relayer.pubkey().as_ref(), 0));
    p.spend_to_address(&relayer, &fresh, &proof, root, nh, 0).expect("the exit is paid");
    assert_eq!(p.balance(&fresh), DENOM);
}
