//! The private bridge channel's pool on Solana (DECISIONS §158).
//!
//! One pool per size (0.1, 1, 10 SOL). A pool is one program account: it
//! holds the SOL and the note tree (the filled subtrees and the last
//! [`ROOTS`] roots). A spent note gets its own empty account, so it cannot
//! be spent twice.
//!
//! - `Deposit`: anyone puts in one note of the pool's size.
//! - `SpendToZyn`: a proof that the caller owns *one of* the notes, bound to
//!   a Zyn deposit code. The SOL stays; the bridge credits SOL.zy on Zyn.
//! - `SpendToAddress`: the same proof, bound to a recipient, a relayer and
//!   the relayer's fee. The pool pays out.
//! - `InsertExit`: the custodians' key adds a note for a private withdrawal
//!   from Zyn, only while the pool holds enough for every note outstanding.
//!
//! Proofs are Groth16 over the `zyn-pool` circuit; the tree uses Solana's
//! Poseidon syscall, which is the circuit's Poseidon.

// `system_instruction`/`system_program` moved to their own crates in 2.3;
// the re-exports still work and keep this program on one dependency.
#![allow(deprecated)]

use groth16_solana::groth16::Groth16Verifier;
use solana_poseidon::{hashv, Endianness, Parameters};
use solana_program::account_info::{next_account_info, AccountInfo};
use solana_program::entrypoint::ProgramResult;
use solana_program::keccak;
use solana_program::log::sol_log_data;
use solana_program::program::{invoke, invoke_signed};
use solana_program::program_error::ProgramError;
use solana_program::pubkey::Pubkey;
use solana_program::rent::Rent;
use solana_program::system_instruction;
use solana_program::sysvar::Sysvar;
use solana_program::{msg, system_program};

mod generated {
    include!("generated/vk.rs");
    include!("generated/zeros.rs");
}
use generated::{SPEND_VK, ZEROS};

#[cfg(not(feature = "no-entrypoint"))]
solana_program::entrypoint!(process);

pub const DEPTH: usize = 20;
pub const ROOTS: usize = 30;

// ------------------------------------------------------------------ state

const MAGIC: [u8; 8] = *b"ZPOOL1\0\0";
const O_AUTHORITY: usize = 8;
const O_DENOM: usize = 40;
const O_NEXT: usize = 48;
const O_ROOT_AT: usize = 52;
const O_LIVE: usize = 56;
const O_BUMP: usize = 64;
const O_FILLED: usize = 72;
const O_ROOTS: usize = O_FILLED + 32 * DEPTH;
pub const STATE_LEN: usize = O_ROOTS + 32 * ROOTS;

/// The pool account for `denomination` lamports.
pub fn pool_address(program: &Pubkey, denomination: u64) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"pool", &denomination.to_le_bytes()], program)
}

/// The account whose existence says `nullifier_hash` was spent.
pub fn nullifier_address(program: &Pubkey, pool: &Pubkey, nullifier_hash: &[u8; 32]) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"nullifier", pool.as_ref(), nullifier_hash], program)
}

fn rd32(d: &[u8], o: usize) -> [u8; 32] {
    d[o..o + 32].try_into().unwrap()
}
fn rd_u64(d: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(d[o..o + 8].try_into().unwrap())
}
fn rd_u32(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(d[o..o + 4].try_into().unwrap())
}

// ----------------------------------------------------------- instructions

pub const IX_INITIALIZE: u8 = 0;
pub const IX_DEPOSIT: u8 = 1;
pub const IX_SPEND_TO_ZYN: u8 = 2;
pub const IX_SPEND_TO_ADDRESS: u8 = 3;
pub const IX_INSERT_EXIT: u8 = 4;

#[derive(Debug)]
#[repr(u32)]
pub enum PoolError {
    NotAPool = 1,
    UnknownRoot,
    BadProof,
    AlreadySpent,
    NotCanonical,
    NotAuthority,
    Underfunded,
    FeeTooHigh,
    TreeFull,
    BadAccount,
}
impl From<PoolError> for ProgramError {
    fn from(e: PoolError) -> Self {
        ProgramError::Custom(e as u32)
    }
}

pub fn process(program: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    let (&tag, rest) = data.split_first().ok_or(ProgramError::InvalidInstructionData)?;
    match tag {
        IX_INITIALIZE => initialize(program, accounts, rest),
        IX_DEPOSIT => deposit(program, accounts, rest),
        IX_SPEND_TO_ZYN => spend_to_zyn(program, accounts, rest),
        IX_SPEND_TO_ADDRESS => spend_to_address(program, accounts, rest),
        IX_INSERT_EXIT => insert_exit(program, accounts, rest),
        _ => Err(ProgramError::InvalidInstructionData),
    }
}

fn arg<const N: usize>(d: &[u8], at: usize) -> Result<[u8; N], ProgramError> {
    d.get(at..at + N).and_then(|s| s.try_into().ok()).ok_or(ProgramError::InvalidInstructionData)
}

/// A pool account of this program, and its denomination.
fn pool_of(program: &Pubkey, pool: &AccountInfo) -> Result<u64, ProgramError> {
    if pool.owner != program || pool.data_len() != STATE_LEN || pool.data.borrow()[..8] != MAGIC {
        return Err(PoolError::NotAPool.into());
    }
    let denom = rd_u64(&pool.data.borrow(), O_DENOM);
    if pool_address(program, denom).0 != *pool.key {
        return Err(PoolError::NotAPool.into());
    }
    Ok(denom)
}

/// `[denomination u64][authority 32]`. Accounts: payer (signer), pool, system.
fn initialize(program: &Pubkey, accounts: &[AccountInfo], d: &[u8]) -> ProgramResult {
    let it = &mut accounts.iter();
    let payer = next_account_info(it)?;
    let pool = next_account_info(it)?;
    let system = next_account_info(it)?;
    let denom = u64::from_le_bytes(arg::<8>(d, 0)?);
    let authority: [u8; 32] = arg(d, 8)?;
    let (addr, bump) = pool_address(program, denom);
    if addr != *pool.key || !payer.is_signer || *system.key != system_program::id() || denom == 0 {
        return Err(PoolError::BadAccount.into());
    }
    let rent = Rent::get()?.minimum_balance(STATE_LEN);
    invoke_signed(
        &system_instruction::create_account(payer.key, pool.key, rent, STATE_LEN as u64, program),
        &[payer.clone(), pool.clone(), system.clone()],
        &[&[b"pool", &denom.to_le_bytes(), &[bump]]],
    )?;
    let mut s = pool.data.borrow_mut();
    s[..8].copy_from_slice(&MAGIC);
    s[O_AUTHORITY..O_AUTHORITY + 32].copy_from_slice(&authority);
    s[O_DENOM..O_DENOM + 8].copy_from_slice(&denom.to_le_bytes());
    s[O_BUMP] = bump;
    for i in 0..DEPTH {
        s[O_FILLED + 32 * i..O_FILLED + 32 * (i + 1)].copy_from_slice(&ZEROS[i]);
    }
    s[O_ROOTS..O_ROOTS + 32].copy_from_slice(&ZEROS[DEPTH]);
    Ok(())
}

fn poseidon2(a: &[u8; 32], b: &[u8; 32]) -> Result<[u8; 32], ProgramError> {
    Ok(hashv(Parameters::Bn254X5, Endianness::BigEndian, &[a, b]).map_err(|_| PoolError::NotCanonical)?.to_bytes())
}

fn canonical(x: &[u8; 32]) -> Result<(), ProgramError> {
    if groth16_solana::groth16::is_less_than_bn254_field_size_be(x) {
        Ok(())
    } else {
        Err(PoolError::NotCanonical.into())
    }
}

/// Append `leaf`; its index and the new root.
fn insert(s: &mut [u8], leaf: [u8; 32]) -> Result<(u32, [u8; 32]), ProgramError> {
    let index = rd_u32(s, O_NEXT);
    if index as usize >= 1 << DEPTH {
        return Err(PoolError::TreeFull.into());
    }
    let mut at = index;
    let mut cur = leaf;
    for i in 0..DEPTH {
        let o = O_FILLED + 32 * i;
        cur = if at % 2 == 0 {
            s[o..o + 32].copy_from_slice(&cur);
            poseidon2(&cur, &ZEROS[i])?
        } else {
            poseidon2(&rd32(s, o), &cur)?
        };
        at /= 2;
    }
    let slot = (rd_u32(s, O_ROOT_AT) + 1) % ROOTS as u32;
    s[O_ROOT_AT..O_ROOT_AT + 4].copy_from_slice(&slot.to_le_bytes());
    let o = O_ROOTS + 32 * slot as usize;
    s[o..o + 32].copy_from_slice(&cur);
    s[O_NEXT..O_NEXT + 4].copy_from_slice(&(index + 1).to_le_bytes());
    Ok((index, cur))
}

fn known_root(s: &[u8], root: &[u8; 32]) -> bool {
    *root != [0u8; 32] && (0..ROOTS).any(|i| rd32(s, O_ROOTS + 32 * i) == *root)
}

fn add_live(s: &mut [u8], delta: i64) {
    let live = (rd_u64(s, O_LIVE) as i64 + delta) as u64;
    s[O_LIVE..O_LIVE + 8].copy_from_slice(&live.to_le_bytes());
}

/// `[commitment 32]`. Accounts: depositor (signer), pool, system.
fn deposit(program: &Pubkey, accounts: &[AccountInfo], d: &[u8]) -> ProgramResult {
    let it = &mut accounts.iter();
    let depositor = next_account_info(it)?;
    let pool = next_account_info(it)?;
    let system = next_account_info(it)?;
    let denom = pool_of(program, pool)?;
    let commitment: [u8; 32] = arg(d, 0)?;
    canonical(&commitment)?;
    if !depositor.is_signer || *system.key != system_program::id() {
        return Err(PoolError::BadAccount.into());
    }
    invoke(&system_instruction::transfer(depositor.key, pool.key, denom), &[depositor.clone(), pool.clone(), system.clone()])?;
    let mut s = pool.data.borrow_mut();
    let (index, root) = insert(&mut s, commitment)?;
    add_live(&mut s, 1);
    sol_log_data(&[b"deposit", &commitment, &index.to_le_bytes(), &root]);
    Ok(())
}

/// Verify a spend and mark its nullifier; the binding is recomputed here
/// from what the instruction says the spend is for.
fn spend<'a>(
    program: &Pubkey,
    payer: &AccountInfo<'a>,
    pool: &AccountInfo<'a>,
    nullifier: &AccountInfo<'a>,
    system: &AccountInfo<'a>,
    proof: &[u8; 256],
    root: &[u8; 32],
    nullifier_hash: &[u8; 32],
    binding: [u8; 32],
) -> ProgramResult {
    pool_of(program, pool)?;
    canonical(root)?;
    canonical(nullifier_hash)?;
    if !payer.is_signer || *system.key != system_program::id() {
        return Err(PoolError::BadAccount.into());
    }
    if !known_root(&pool.data.borrow(), root) {
        return Err(PoolError::UnknownRoot.into());
    }
    let (addr, bump) = nullifier_address(program, pool.key, nullifier_hash);
    if addr != *nullifier.key {
        return Err(PoolError::BadAccount.into());
    }
    if nullifier.lamports() > 0 {
        return Err(PoolError::AlreadySpent.into());
    }
    let a: [u8; 64] = proof[..64].try_into().unwrap();
    let b: [u8; 128] = proof[64..192].try_into().unwrap();
    let c: [u8; 64] = proof[192..].try_into().unwrap();
    let inputs = [*root, *nullifier_hash, binding];
    let ok = Groth16Verifier::new(&a, &b, &c, &inputs, &SPEND_VK).map(|mut v| v.verify().is_ok()).unwrap_or(false);
    if !ok {
        msg!("the proof does not verify");
        return Err(PoolError::BadProof.into());
    }
    let rent = Rent::get()?.minimum_balance(0);
    invoke_signed(
        &system_instruction::create_account(payer.key, nullifier.key, rent, 0, program),
        &[payer.clone(), nullifier.clone(), system.clone()],
        &[&[b"nullifier", pool.key.as_ref(), nullifier_hash, &[bump]]],
    )?;
    add_live(&mut pool.data.borrow_mut(), -1);
    Ok(())
}

fn binding(domain: &[u8], data: &[u8]) -> [u8; 32] {
    let mut h = keccak::hashv(&[domain, data]).to_bytes();
    h[0] = 0;
    h
}

/// `[proof 256][root 32][nullifier_hash 32][deposit_code 32]`.
/// Accounts: payer (signer), pool, nullifier, system.
fn spend_to_zyn(program: &Pubkey, accounts: &[AccountInfo], d: &[u8]) -> ProgramResult {
    let it = &mut accounts.iter();
    let (payer, pool, nullifier, system) = (next_account_info(it)?, next_account_info(it)?, next_account_info(it)?, next_account_info(it)?);
    let proof: [u8; 256] = arg(d, 0)?;
    let root: [u8; 32] = arg(d, 256)?;
    let nh: [u8; 32] = arg(d, 288)?;
    let code: [u8; 32] = arg(d, 320)?;
    spend(program, payer, pool, nullifier, system, &proof, &root, &nh, binding(b"zyn-pool.to-zyn.v1", &code))?;
    sol_log_data(&[b"to_zyn", &nh, &code]);
    Ok(())
}

/// `[proof 256][root 32][nullifier_hash 32][fee u64]`.
/// Accounts: relayer (signer, paid the fee), pool, nullifier, recipient, system.
fn spend_to_address(program: &Pubkey, accounts: &[AccountInfo], d: &[u8]) -> ProgramResult {
    let it = &mut accounts.iter();
    let (relayer, pool, nullifier, recipient, system) =
        (next_account_info(it)?, next_account_info(it)?, next_account_info(it)?, next_account_info(it)?, next_account_info(it)?);
    let proof: [u8; 256] = arg(d, 0)?;
    let root: [u8; 32] = arg(d, 256)?;
    let nh: [u8; 32] = arg(d, 288)?;
    let fee = u64::from_le_bytes(arg::<8>(d, 320)?);
    let denom = pool_of(program, pool)?;
    if fee > denom {
        return Err(PoolError::FeeTooHigh.into());
    }
    let mut data = Vec::with_capacity(74);
    data.extend_from_slice(&32u16.to_be_bytes());
    data.extend_from_slice(recipient.key.as_ref());
    data.extend_from_slice(relayer.key.as_ref());
    data.extend_from_slice(&fee.to_be_bytes());
    spend(program, relayer, pool, nullifier, system, &proof, &root, &nh, binding(b"zyn-pool.to-address.v1", &data))?;
    **pool.try_borrow_mut_lamports()? -= denom;
    **recipient.try_borrow_mut_lamports()? += denom - fee;
    **relayer.try_borrow_mut_lamports()? += fee;
    sol_log_data(&[b"to_address", &nh]);
    Ok(())
}

/// `[commitment 32]`. Accounts: authority (signer), pool.
fn insert_exit(program: &Pubkey, accounts: &[AccountInfo], d: &[u8]) -> ProgramResult {
    let it = &mut accounts.iter();
    let authority = next_account_info(it)?;
    let pool = next_account_info(it)?;
    let denom = pool_of(program, pool)?;
    let commitment: [u8; 32] = arg(d, 0)?;
    canonical(&commitment)?;
    let mut s = pool.data.borrow_mut();
    if !authority.is_signer || rd32(&s, O_AUTHORITY) != authority.key.to_bytes() {
        return Err(PoolError::NotAuthority.into());
    }
    // Every note in the tree must stay payable: the pool holds its rent plus
    // one size for each note not yet spent, and this one.
    let need = Rent::get()?.minimum_balance(STATE_LEN) as u128 + denom as u128 * (rd_u64(&s, O_LIVE) as u128 + 1);
    if (pool.lamports() as u128) < need {
        return Err(PoolError::Underfunded.into());
    }
    let (index, root) = insert(&mut s, commitment)?;
    add_live(&mut s, 1);
    sol_log_data(&[b"exit", &commitment, &index.to_le_bytes(), &root]);
    Ok(())
}
