//! A Squads v4 multisig as the SOL vault's signer (DECISIONS §171): the
//! Solana counterpart of the ETH bridge's Safe mode (§155).
//!
//! The vault is the multisig's vault PDA, so no key the bridge holds can move
//! what it holds. The bridge holds a *proposer* — a member with only the
//! Initiate permission — and turns each payout batch into a vault transaction
//! plus its proposal, in one Solana transaction. The owners approve and
//! execute it in the Squads app; the bridge settles the exits once the
//! execution is final.
//!
//! Everything here is bytes: the instructions as the Squads program decodes
//! them (`programs/squads_multisig_program` at v4), and the two accounts the
//! bridge reads back, the multisig and a proposal.

use crate::sol_pool::{Ix, Meta, Pubkey, SYSTEM_PROGRAM};
use zyn_bridge::solana::{associated_token_address, find_program_address, SolPayout, ATA_PROGRAM, TOKEN_PROGRAM};

/// `SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf`, the same on mainnet and devnet.
pub const PROGRAM: &str = "SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf";

/// The member permissions, as a mask.
pub const INITIATE: u8 = 1;
pub const VOTE: u8 = 2;
pub const EXECUTE: u8 = 4;

pub fn program() -> Pubkey {
    zyn_custody::solana::pubkey(PROGRAM).expect("the Squads program id is base58")
}

/// An Anchor instruction's tag: the first 8 bytes of sha256("global:<name>").
pub fn discriminator(name: &str) -> [u8; 8] {
    use sha2::Digest as _;
    let h = sha2::Sha256::digest(format!("global:{name}").as_bytes());
    h[..8].try_into().unwrap()
}

pub fn vault_pda(multisig: &Pubkey, index: u8) -> Pubkey {
    find_program_address(&[b"multisig", multisig, b"vault", &[index]], &program()).0
}

pub fn transaction_pda(multisig: &Pubkey, index: u64) -> Pubkey {
    find_program_address(&[b"multisig", multisig, b"transaction", &index.to_le_bytes()], &program()).0
}

pub fn proposal_pda(multisig: &Pubkey, index: u64) -> Pubkey {
    find_program_address(&[b"multisig", multisig, b"transaction", &index.to_le_bytes(), b"proposal"], &program()).0
}

/// The instructions a payout batch is: a transfer per SOL payout, and per
/// token payout the recipient's token account (made if missing, paid by the
/// vault) and a transfer into it — the same as the custodians' message
/// (`zyn_bridge::solana::message`), without the nonce advance: a Squads
/// transaction executes once, by its index.
pub fn payout_ixs(vault: &Pubkey, payouts: &[SolPayout]) -> Result<Vec<Ix>, String> {
    let mut out = Vec::new();
    for p in payouts {
        if p.to == *vault {
            return Err("a payout to the vault itself".into());
        }
        match p.token {
            None => {
                if p.lamports == 0 {
                    return Err("a payout of nothing".into());
                }
                let mut data = 2u32.to_le_bytes().to_vec();
                data.extend_from_slice(&p.lamports.to_le_bytes());
                out.push(Ix { program: SYSTEM_PROGRAM, accounts: vec![Meta { key: *vault, signer: true, writable: true }, Meta { key: p.to, signer: false, writable: true }], data });
            }
            Some((mint, amount)) => {
                if amount == 0 {
                    return Err("a payout of nothing".into());
                }
                let dest = associated_token_address(&p.to, &mint);
                let src = associated_token_address(vault, &mint);
                out.push(Ix {
                    program: ATA_PROGRAM,
                    accounts: vec![
                        Meta { key: *vault, signer: true, writable: true },
                        Meta { key: dest, signer: false, writable: true },
                        Meta { key: p.to, signer: false, writable: false },
                        Meta { key: mint, signer: false, writable: false },
                        Meta { key: SYSTEM_PROGRAM, signer: false, writable: false },
                        Meta { key: TOKEN_PROGRAM, signer: false, writable: false },
                    ],
                    data: vec![1],
                });
                let mut data = vec![3u8];
                data.extend_from_slice(&amount.to_le_bytes());
                out.push(Ix {
                    program: TOKEN_PROGRAM,
                    accounts: vec![Meta { key: src, signer: false, writable: true }, Meta { key: dest, signer: false, writable: true }, Meta { key: *vault, signer: true, writable: false }],
                    data,
                });
            }
        }
    }
    Ok(out)
}

/// Squads' `TransactionMessage`: what the vault will sign when executed.
/// Keys are the vault (the one signer) first, then accounts written, then
/// accounts only read (the programs among them); lengths are one byte, except
/// an instruction's data, which is two.
pub fn transaction_message(vault: &Pubkey, ixs: &[Ix]) -> Result<Vec<u8>, String> {
    let mut keys: Vec<(Pubkey, bool)> = vec![(*vault, true)];
    let mut add = |k: Pubkey, w: bool| {
        if let Some(e) = keys.iter_mut().find(|e| e.0 == k) {
            e.1 |= w;
        } else {
            keys.push((k, w));
        }
    };
    for ix in ixs {
        for m in &ix.accounts {
            if m.signer && m.key != *vault {
                return Err("only the vault signs a vault transaction".into());
            }
            add(m.key, m.writable);
        }
        add(ix.program, false);
    }
    let vault_entry = keys.remove(0);
    keys.sort_by_key(|&(_, w)| !w);
    keys.insert(0, vault_entry);
    if keys.len() > u8::MAX as usize || ixs.len() > u8::MAX as usize {
        return Err("too many accounts or instructions for one vault transaction".into());
    }
    let at = |k: &Pubkey| keys.iter().position(|e| &e.0 == k).expect("every key is listed") as u8;
    let writable_non_signers = keys.iter().skip(1).filter(|k| k.1).count();
    let mut out = vec![1u8, 1u8, writable_non_signers as u8, keys.len() as u8];
    for k in &keys {
        out.extend_from_slice(&k.0);
    }
    out.push(ixs.len() as u8);
    for ix in ixs {
        out.push(at(&ix.program));
        out.push(u8::try_from(ix.accounts.len()).map_err(|_| "an instruction with too many accounts")?);
        for m in &ix.accounts {
            out.push(at(&m.key));
        }
        out.extend_from_slice(&u16::try_from(ix.data.len()).map_err(|_| "an instruction's data is too long")?.to_le_bytes());
        out.extend_from_slice(&ix.data);
    }
    out.push(0); // no address table lookups
    Ok(out)
}

/// `vault_transaction_create`: the batch, stored under the multisig's next
/// transaction index.
pub fn vault_transaction_create(multisig: &Pubkey, index: u64, member: &Pubkey, vault_index: u8, message: &[u8], memo: &str) -> Ix {
    let mut data = discriminator("vault_transaction_create").to_vec();
    data.push(vault_index);
    data.push(0); // no ephemeral signers
    data.extend_from_slice(&(message.len() as u32).to_le_bytes());
    data.extend_from_slice(message);
    data.push(1);
    data.extend_from_slice(&(memo.len() as u32).to_le_bytes());
    data.extend_from_slice(memo.as_bytes());
    Ix {
        program: program(),
        accounts: vec![
            Meta { key: *multisig, signer: false, writable: true },
            Meta { key: transaction_pda(multisig, index), signer: false, writable: true },
            Meta { key: *member, signer: true, writable: false },
            Meta { key: *member, signer: true, writable: true },
            Meta { key: SYSTEM_PROGRAM, signer: false, writable: false },
        ],
        data,
    }
}

/// `proposal_create`, active at once: the owners vote on it.
pub fn proposal_create(multisig: &Pubkey, index: u64, member: &Pubkey) -> Ix {
    let mut data = discriminator("proposal_create").to_vec();
    data.extend_from_slice(&index.to_le_bytes());
    data.push(0); // not a draft
    Ix {
        program: program(),
        accounts: vec![
            Meta { key: *multisig, signer: false, writable: false },
            Meta { key: proposal_pda(multisig, index), signer: false, writable: true },
            Meta { key: *member, signer: true, writable: false },
            Meta { key: *member, signer: true, writable: true },
            Meta { key: SYSTEM_PROGRAM, signer: false, writable: false },
        ],
        data,
    }
}

/// `proposal_approve`: the member's vote for a proposal.
pub fn proposal_approve(multisig: &Pubkey, index: u64, member: &Pubkey) -> Ix {
    let mut data = discriminator("proposal_approve").to_vec();
    data.push(0); // no memo
    Ix {
        program: program(),
        accounts: vec![
            Meta { key: *multisig, signer: false, writable: false },
            Meta { key: *member, signer: true, writable: true },
            Meta { key: proposal_pda(multisig, index), signer: false, writable: true },
        ],
        data,
    }
}

/// `vault_transaction_execute` for an approved batch: the stored message's
/// accounts follow, in its order, none of them signing here — the vault
/// signs inside the program.
pub fn vault_transaction_execute(multisig: &Pubkey, index: u64, member: &Pubkey, keys: &[(Pubkey, bool)]) -> Ix {
    let mut accounts = vec![
        Meta { key: *multisig, signer: false, writable: false },
        Meta { key: proposal_pda(multisig, index), signer: false, writable: true },
        Meta { key: transaction_pda(multisig, index), signer: false, writable: false },
        Meta { key: *member, signer: true, writable: false },
    ];
    accounts.extend(keys.iter().map(|&(key, writable)| Meta { key, signer: false, writable }));
    Ix { program: program(), accounts, data: discriminator("vault_transaction_execute").to_vec() }
}

/// A stored vault transaction's accounts, in its order, each with whether it
/// is written: what executing it must pass. None for one that uses address
/// tables (this bridge never makes one) or does not decode.
pub fn vault_transaction_keys(d: &[u8]) -> Option<Vec<(Pubkey, bool)>> {
    use sha2::Digest as _;
    if d.get(..8)? != &sha2::Sha256::digest(b"account:VaultTransaction")[..8] {
        return None;
    }
    let u32_at = |o: usize| Some(u32::from_le_bytes(d.get(o..o + 4)?.try_into().ok()?) as usize);
    // multisig, creator, index, bump, vault_index, vault_bump
    let mut o = 8 + 32 + 32 + 8 + 3;
    o += 4 + u32_at(o)?; // ephemeral signer bumps
    let (signers, writable_signers, writable_others) = (*d.get(o)? as usize, *d.get(o + 1)? as usize, *d.get(o + 2)? as usize);
    o += 3;
    let n = u32_at(o)?;
    o += 4;
    let mut keys = Vec::with_capacity(n.min(256));
    for i in 0..n {
        let k: Pubkey = d.get(o..o + 32)?.try_into().ok()?;
        o += 32;
        let writable = if i < signers { i < writable_signers } else { i < signers + writable_others };
        keys.push((k, writable));
    }
    // Instructions, then the address table lookups, which must be none.
    let ixs = u32_at(o)?;
    o += 4;
    for _ in 0..ixs {
        o += 1; // program index
        o += 4 + u32_at(o)?; // account indexes
        o += 4 + u32_at(o)?; // data
    }
    (u32_at(o)? == 0).then_some(keys)
}

/// What the bridge reads from the multisig account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Multisig {
    pub threshold: u16,
    pub time_lock: u32,
    pub transaction_index: u64,
    pub stale_transaction_index: u64,
    pub rent_collector: Option<Pubkey>,
    pub members: Vec<(Pubkey, u8)>,
}

impl Multisig {
    pub fn decode(d: &[u8]) -> Option<Multisig> {
        if d.get(..8)? != Self::account_tag() {
            return None;
        }
        let mut o = 8 + 32 + 32; // create_key, config_authority
        let u16_at = |o: usize| Some(u16::from_le_bytes(d.get(o..o + 2)?.try_into().ok()?));
        let u32_at = |o: usize| Some(u32::from_le_bytes(d.get(o..o + 4)?.try_into().ok()?));
        let u64_at = |o: usize| Some(u64::from_le_bytes(d.get(o..o + 8)?.try_into().ok()?));
        let threshold = u16_at(o)?;
        let time_lock = u32_at(o + 2)?;
        let transaction_index = u64_at(o + 6)?;
        let stale_transaction_index = u64_at(o + 14)?;
        o += 22;
        let rent_collector = match *d.get(o)? {
            0 => {
                o += 1;
                None
            }
            1 => {
                let k: Pubkey = d.get(o + 1..o + 33)?.try_into().ok()?;
                o += 33;
                Some(k)
            }
            _ => return None,
        };
        o += 1; // bump
        let n = u32_at(o)? as usize;
        o += 4;
        let mut members = Vec::with_capacity(n.min(64));
        for _ in 0..n {
            let k: Pubkey = d.get(o..o + 32)?.try_into().ok()?;
            members.push((k, *d.get(o + 32)?));
            o += 33;
        }
        Some(Multisig { threshold, time_lock, transaction_index, stale_transaction_index, rent_collector, members })
    }

    /// Anchor's account tag: sha256("account:Multisig")[..8].
    fn account_tag() -> [u8; 8] {
        use sha2::Digest as _;
        sha2::Sha256::digest(b"account:Multisig")[..8].try_into().unwrap()
    }

    pub fn permissions(&self, key: &Pubkey) -> Option<u8> {
        self.members.iter().find(|m| &m.0 == key).map(|m| m.1)
    }

    /// Owners: members who can vote.
    pub fn voters(&self) -> usize {
        self.members.iter().filter(|m| m.1 & VOTE != 0).count()
    }
}

/// A proposal's status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Draft,
    Active,
    Rejected,
    Approved,
    Executing,
    Executed,
    Cancelled,
}

/// A proposal account's status and the transaction index it is for.
pub fn decode_proposal(d: &[u8]) -> Option<(u64, Status)> {
    use sha2::Digest as _;
    if d.get(..8)? != &sha2::Sha256::digest(b"account:Proposal")[..8] {
        return None;
    }
    let index = u64::from_le_bytes(d.get(40..48)?.try_into().ok()?);
    let status = match *d.get(48)? {
        0 => Status::Draft,
        1 => Status::Active,
        2 => Status::Rejected,
        3 => Status::Approved,
        4 => Status::Executing,
        5 => Status::Executed,
        6 => Status::Cancelled,
        _ => return None,
    };
    Some((index, status))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(s: &str) -> Pubkey {
        zyn_custody::solana::pubkey(s).unwrap()
    }

    #[test]
    fn the_teams_multisig_derives_its_vault() {
        // The mainnet multisig the team made (6 Oct 2026) and the vault the
        // Squads app shows for it.
        let ms = key("A334BUsBudh3RzkSpxgKu3wcCsyVYk4iP2iXMk9pFSvh");
        assert_eq!(zyn_custody::solana::base58_encode(&vault_pda(&ms, 0)), "23bHgTpb8kcbBXcKP66MdjrRfM4UgAtjDDFkidWYLwnU");
    }

    #[test]
    fn instruction_tags_are_anchors() {
        // As found in a VaultTransactionCreate on mainnet.
        assert_eq!(discriminator("vault_transaction_create"), [0x30, 0xfa, 0x4e, 0xa8, 0xd0, 0xe2, 0xda, 0xd3]);
    }

    #[test]
    fn the_teams_multisig_decodes() {
        // Its account as read from mainnet on 6 Oct 2026.
        let raw = include_bytes!("../tests/data/squads-multisig.bin");
        let m = Multisig::decode(raw).unwrap();
        assert_eq!((m.threshold, m.time_lock, m.transaction_index, m.stale_transaction_index, m.rent_collector), (2, 0, 0, 0, None));
        assert_eq!(m.members.len(), 3);
        assert!(m.members.iter().all(|x| x.1 == INITIATE | VOTE | EXECUTE));
        assert_eq!(m.voters(), 3);
        assert_eq!(m.permissions(&key("7RTAnEokPmzycm5q3cwWZV9VXGv6KV3g8LEsfgeyJ7RK")), Some(7));
        // The account has room to spare past its members (231 bytes, 191
        // used); cut into the last member, it does not decode.
        assert!(Multisig::decode(&raw[..190]).is_none());
    }

    #[test]
    fn a_message_lists_the_vault_then_written_then_read() {
        let vault = [9u8; 32];
        let (a, b) = ([1u8; 32], [2u8; 32]);
        let ixs = payout_ixs(&vault, &[SolPayout::native(a, 5), SolPayout::native(b, 7), SolPayout::native(a, 1)]).unwrap();
        let m = transaction_message(&vault, &ixs).unwrap();
        // signers 1, writable signers 1, writable non-signers 2, 4 keys.
        assert_eq!(&m[..4], &[1, 1, 2, 4]);
        assert_eq!(&m[4..36], &vault);
        assert_eq!(&m[36..68], &a);
        assert_eq!(&m[68..100], &b);
        assert_eq!(&m[100..132], &SYSTEM_PROGRAM);
        // 3 instructions: system(3) [vault, a] data 12 = transfer 5.
        assert_eq!(&m[132..139], &[3, 3, 2, 0, 1, 12, 0]);
        assert_eq!(&m[139..151], &[2, 0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(*m.last().unwrap(), 0);
        assert!(payout_ixs(&vault, &[SolPayout::native(vault, 1)]).is_err());
        assert!(payout_ixs(&vault, &[SolPayout::native(a, 0)]).is_err());
    }

    #[test]
    fn a_stored_batch_lists_its_accounts_for_execution() {
        use sha2::Digest as _;
        let vault = [9u8; 32];
        let (a, b) = ([1u8; 32], [2u8; 32]);
        let ixs = payout_ixs(&vault, &[SolPayout::native(a, 5), SolPayout::native(b, 7)]).unwrap();
        let m = transaction_message(&vault, &ixs).unwrap();
        // As the program stores it: header, then the message with u32 lengths.
        let mut d = sha2::Sha256::digest(b"account:VaultTransaction")[..8].to_vec();
        d.extend_from_slice(&[0u8; 32 + 32 + 8 + 3]);
        d.extend_from_slice(&0u32.to_le_bytes());
        d.extend_from_slice(&m[..3]);
        d.extend_from_slice(&(m[3] as u32).to_le_bytes());
        d.extend_from_slice(&m[4..4 + 32 * m[3] as usize]);
        d.extend_from_slice(&2u32.to_le_bytes());
        for _ in 0..2 {
            d.push(2);
            d.extend_from_slice(&2u32.to_le_bytes());
            d.extend_from_slice(&[0, 1]);
            d.extend_from_slice(&12u32.to_le_bytes());
            d.extend_from_slice(&[0u8; 12]);
        }
        d.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(vault_transaction_keys(&d).unwrap(), vec![(vault, true), (a, true), (b, true), (SYSTEM_PROGRAM, false)]);
        let ix = vault_transaction_execute(&[3u8; 32], 2, &[4u8; 32], &vault_transaction_keys(&d).unwrap());
        assert_eq!(ix.accounts.len(), 8);
        assert!(ix.accounts[4..].iter().all(|m| !m.signer));
        // An address table would need accounts this bridge does not pass.
        let last = d.len() - 4;
        d[last] = 1;
        assert!(vault_transaction_keys(&d).is_none());
    }

    #[test]
    fn a_proposal_status_decodes() {
        use sha2::Digest as _;
        let mut d = sha2::Sha256::digest(b"account:Proposal")[..8].to_vec();
        d.extend_from_slice(&[7u8; 32]);
        d.extend_from_slice(&5u64.to_le_bytes());
        d.push(5);
        d.extend_from_slice(&0i64.to_le_bytes());
        assert_eq!(decode_proposal(&d), Some((5, Status::Executed)));
        d[48] = 2;
        assert_eq!(decode_proposal(&d), Some((5, Status::Rejected)));
        d[0] ^= 1;
        assert_eq!(decode_proposal(&d), None);
    }
}
