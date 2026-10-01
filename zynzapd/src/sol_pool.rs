//! The private channel's Solana side, as the SOL bridge runs it (DECISIONS
//! §158): plain Solana transactions signed by one key, the pools' notes read
//! back from their logs, and the relayer that submits spends so no wallet of
//! the holder's pays for — and so shows up beside — a private spend.
//!
//! On testnet one key is both the pools' exit authority and the relayer's
//! fee payer; the custodians' 2-of-3 key takes the authority over with the
//! program's `SetAuthority` (§158.4).

use serde_json::{json, Value};

pub type Pubkey = [u8; 32];

/// `ComputeBudget111111111111111111111111111111`.
pub const COMPUTE_BUDGET: Pubkey = [
    3, 6, 70, 111, 229, 33, 23, 50, 255, 236, 173, 186, 114, 195, 155, 231, 188, 140, 229, 187, 197, 247, 18, 107, 44, 67, 155, 58, 64, 0, 0, 0,
];
pub const SYSTEM_PROGRAM: Pubkey = [0u8; 32];

/// The pool program's instructions, and its account layout's facts.
pub const IX_SPEND_TO_ZYN: u8 = 2;
pub const IX_SPEND_TO_ADDRESS: u8 = 3;
pub const IX_INSERT_EXIT: u8 = 4;
pub const POOL_STATE_LEN: u64 = 1672;
const O_LIVE: usize = 56;

pub struct Meta {
    pub key: Pubkey,
    pub signer: bool,
    pub writable: bool,
}

pub struct Ix {
    pub program: Pubkey,
    pub accounts: Vec<Meta>,
    pub data: Vec<u8>,
}

fn compact(n: usize, out: &mut Vec<u8>) {
    let mut n = n;
    loop {
        let mut b = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            out.push(b);
            return;
        }
        b |= 0x80;
        out.push(b);
    }
}

/// A legacy message: the payer first, then signers that write, signers that
/// only read, accounts that are written, accounts that are only read (the
/// programs among them), as the runtime requires.
pub fn message(payer: &Pubkey, ixs: &[Ix], blockhash: &[u8; 32]) -> Vec<u8> {
    let mut keys: Vec<(Pubkey, bool, bool)> = vec![(*payer, true, true)];
    let mut add = |k: Pubkey, s: bool, w: bool| {
        if let Some(e) = keys.iter_mut().find(|e| e.0 == k) {
            e.1 |= s;
            e.2 |= w;
        } else {
            keys.push((k, s, w));
        }
    };
    for ix in ixs {
        for m in &ix.accounts {
            add(m.key, m.signer, m.writable);
        }
        add(ix.program, false, false);
    }
    let payer_entry = keys.remove(0);
    keys.sort_by_key(|&(_, s, w)| match (s, w) {
        (true, true) => 0,
        (true, false) => 1,
        (false, true) => 2,
        (false, false) => 3,
    });
    keys.insert(0, payer_entry);
    let signed = keys.iter().filter(|k| k.1).count();
    let ro_signed = keys.iter().filter(|k| k.1 && !k.2).count();
    let ro_unsigned = keys.iter().filter(|k| !k.1 && !k.2).count();
    let at = |k: &Pubkey| keys.iter().position(|e| &e.0 == k).expect("every key is listed") as u8;
    let mut out = vec![signed as u8, ro_signed as u8, ro_unsigned as u8];
    compact(keys.len(), &mut out);
    for k in &keys {
        out.extend_from_slice(&k.0);
    }
    out.extend_from_slice(blockhash);
    compact(ixs.len(), &mut out);
    for ix in ixs {
        out.push(at(&ix.program));
        compact(ix.accounts.len(), &mut out);
        for m in &ix.accounts {
            out.push(at(&m.key));
        }
        compact(ix.data.len(), &mut out);
        out.extend_from_slice(&ix.data);
    }
    out
}

pub fn compute_limit(units: u32) -> Ix {
    let mut data = vec![2u8];
    data.extend_from_slice(&units.to_le_bytes());
    Ix { program: COMPUTE_BUDGET, accounts: vec![], data }
}

/// The single key the pools' authority and the relayer use on testnet: a
/// Solana CLI keypair file (64 bytes: secret, then public).
pub struct Signer {
    key: ed25519_dalek::SigningKey,
}

impl Signer {
    pub fn load(path: &str) -> Result<Signer, String> {
        let raw = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        let bytes: Vec<u8> = serde_json::from_str(&raw).map_err(|_| format!("{path} is not a keypair file"))?;
        let secret: [u8; 32] = bytes.get(..32).and_then(|b| b.try_into().ok()).ok_or_else(|| format!("{path} is not a keypair file"))?;
        let key = ed25519_dalek::SigningKey::from_bytes(&secret);
        if bytes.get(32..64) != Some(&key.verifying_key().to_bytes()[..]) {
            return Err(format!("{path}: its public half does not match its secret"));
        }
        Ok(Signer { key })
    }

    pub fn pubkey(&self) -> Pubkey {
        self.key.verifying_key().to_bytes()
    }

    /// 32 bytes only this key can derive for `label`: a seed for an account
    /// this key's service makes, the same every time it is asked.
    pub fn derive(&self, label: &[u8]) -> [u8; 32] {
        use sha2::Digest as _;
        let mut h = sha2::Sha256::new();
        h.update(b"zyn.signer-derive.v1\0");
        h.update(self.key.to_bytes());
        h.update(label);
        h.finalize().into()
    }

    /// This key's signature over `msg` (one of several on a transaction).
    pub fn sign(&self, msg: &[u8]) -> [u8; 64] {
        use ed25519_dalek::Signer as _;
        self.key.sign(msg).to_bytes()
    }

    /// One signed transaction, payer = this key.
    pub fn transaction(&self, ixs: &[Ix], blockhash: &[u8; 32]) -> (String, Vec<u8>) {
        use ed25519_dalek::Signer as _;
        let msg = message(&self.pubkey(), ixs, blockhash);
        let sig = self.key.sign(&msg).to_bytes();
        (zyn_custody::solana::base58_encode(&sig), zyn_bridge::solana::transaction(&msg, &sig))
    }
}

/// How many notes a pool still owes (unspent): its `live` counter.
pub fn live_notes(account_data: &[u8]) -> Option<u64> {
    Some(u64::from_le_bytes(account_data.get(O_LIVE..O_LIVE + 8)?.try_into().ok()?))
}

/// The instruction inserting an exit note.
pub fn insert_exit(program: &Pubkey, authority: &Pubkey, pool: &Pubkey, commitment: &[u8; 32]) -> Ix {
    let mut data = vec![IX_INSERT_EXIT];
    data.extend_from_slice(commitment);
    Ix { program: *program, accounts: vec![Meta { key: *authority, signer: true, writable: false }, Meta { key: *pool, signer: false, writable: true }], data }
}

/// A spend, relayed: into Zyn under `code`, or to `recipient` for `fee`.
pub fn spend(program: &Pubkey, relayer: &Pubkey, pool: &Pubkey, proof: &[u8; 256], root: &[u8; 32], nullifier_hash: &[u8; 32], to: &SpendTo) -> Ix {
    let nullifier = zyn_bridge::solana::find_program_address(&[b"nullifier", pool, nullifier_hash], program).0;
    let mut data = vec![match to {
        SpendTo::Zyn { .. } => IX_SPEND_TO_ZYN,
        SpendTo::Address { .. } => IX_SPEND_TO_ADDRESS,
    }];
    data.extend_from_slice(proof);
    data.extend_from_slice(root);
    data.extend_from_slice(nullifier_hash);
    let mut accounts = vec![
        Meta { key: *relayer, signer: true, writable: true },
        Meta { key: *pool, signer: false, writable: true },
        Meta { key: nullifier, signer: false, writable: true },
    ];
    match to {
        SpendTo::Zyn { code } => data.extend_from_slice(code),
        SpendTo::Address { recipient, fee } => {
            data.extend_from_slice(&fee.to_le_bytes());
            accounts.push(Meta { key: *recipient, signer: false, writable: true });
        }
    }
    accounts.push(Meta { key: SYSTEM_PROGRAM, signer: false, writable: false });
    Ix { program: *program, accounts, data }
}

pub enum SpendTo {
    Zyn { code: [u8; 32] },
    Address { recipient: Pubkey, fee: u64 },
}

/// A pool's notes in order, rebuilt from its `deposit` and `exit` events
/// (each carries the note and its index), and the nullifier hashes of its
/// spends — so a wallet restored from its phrase can tell which of its notes
/// are spent. Kept in a file between calls.
#[derive(Default)]
pub struct Leaves {
    pub newest: Option<String>,
    pub leaves: Vec<String>,
    pub spent: Vec<String>,
}

impl Leaves {
    pub fn load(path: &std::path::Path) -> Leaves {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .map(|v| Leaves {
                newest: v["newest"].as_str().map(String::from),
                leaves: v["leaves"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect(),
                spent: v["spent"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect(),
            })
            .unwrap_or_default()
    }

    pub fn save(&self, path: &std::path::Path) -> Result<(), String> {
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, json!({ "newest": self.newest, "leaves": self.leaves, "spent": self.spent }).to_string()).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())
    }

    /// Place a note at its index; an index already filled with another note
    /// means the file and the chain disagree, which stops the index.
    pub fn place(&mut self, index: usize, note: String) -> Result<(), String> {
        if index < self.leaves.len() {
            return if self.leaves[index] == note { Ok(()) } else { Err(format!("leaf {index} differs from the chain")) };
        }
        if index != self.leaves.len() {
            return Err(format!("leaf {index} arrived before leaf {}", self.leaves.len()));
        }
        self.leaves.push(note);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_lengths_are_the_runtimes() {
        let enc = |n| {
            let mut v = Vec::new();
            compact(n, &mut v);
            v
        };
        assert_eq!(enc(0), vec![0]);
        assert_eq!(enc(127), vec![0x7f]);
        assert_eq!(enc(128), vec![0x80, 0x01]);
        assert_eq!(enc(16383), vec![0xff, 0x7f]);
    }

    #[test]
    fn a_message_orders_its_accounts_as_the_runtime_requires() {
        let (payer, pool, recipient, program) = ([1u8; 32], [2u8; 32], [3u8; 32], [9u8; 32]);
        let ix = spend(&program, &payer, &pool, &[0u8; 256], &[4u8; 32], &[5u8; 32], &SpendTo::Address { recipient, fee: 7 });
        let msg = message(&payer, &[compute_limit(400_000), ix], &[8u8; 32]);
        // One signer (the payer, who writes), no read-only signer, and the
        // two programs plus the system program read-only.
        assert_eq!(&msg[..3], &[1, 0, 3]);
        let n = msg[3] as usize;
        let keys: Vec<&[u8]> = (0..n).map(|i| &msg[4 + 32 * i..4 + 32 * (i + 1)]).collect();
        assert_eq!(keys[0], &payer[..], "the payer comes first");
        let last3: Vec<&[u8]> = keys[n - 3..].to_vec();
        assert!(last3.contains(&&COMPUTE_BUDGET[..]) && last3.contains(&&program[..]) && last3.contains(&&SYSTEM_PROGRAM[..]), "programs are read-only, at the end");
        assert!(keys[1..n - 3].contains(&&pool[..]) && keys[1..n - 3].contains(&&recipient[..]), "written accounts before read-only ones");
    }

    #[test]
    fn leaves_are_placed_by_index_and_a_disagreement_stops_them() {
        let mut l = Leaves::default();
        l.place(0, "a".into()).unwrap();
        l.place(0, "a".into()).unwrap();
        l.place(1, "b".into()).unwrap();
        assert!(l.place(1, "c".into()).is_err(), "the chain said b");
        assert!(l.place(3, "d".into()).is_err(), "a gap");
        assert_eq!(l.leaves, vec!["a", "b"]);
    }
}
