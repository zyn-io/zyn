//! ETH.zy's bridge (DECISIONS §146): the vault's digest, its withdrawal call,
//! its deposit log, and a signed transaction — in the contract's encoding,
//! byte for byte.
//!
//! The vault is `contracts/src/ZynVault.sol` with a `ZynSigners` 2-of-3 as
//! its signer. A deposit is `depositNative(commitment, asset)`: the `bytes32`
//! the contract calls a destination carries Nap's commitment, so it names no
//! Zyn account. A withdrawal is a list of payouts signed by two owners, in
//! ascending address order, over the vault's EIP-712 digest — computed here
//! and cross-checked against the contract's own `withdrawalDigest` before any
//! key signs it.

use k256::ecdsa::{RecoveryId, Signature, SigningKey};
use zyn_vm::commit::{Encoder, Hash};
use zyn_vm::eip712::keccak;

pub type Address20 = [u8; 20];

/// `keccak256("Withdrawal(uint256 nonce,uint256 epoch,Payout[] payouts)Payout(address to,uint256 asset,uint256 amount)")`
pub const WITHDRAWAL_TYPEHASH: &str = "4bbec971ea7bfa995baa1756a56c4ea39f88b2b17e6e7a9813e430ce708de506";
/// `keccak256("Payout(address to,uint256 asset,uint256 amount)")`
pub const PAYOUT_TYPEHASH: &str = "3fbf17ffac88b99d8e834e3fbf5378bbf17176b75d0466005ce02d6d0b1a3854";

fn h32(s: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).expect("constant hex");
    }
    out
}

fn word_u(v: u128) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[16..].copy_from_slice(&v.to_be_bytes());
    w
}

fn word_addr(a: &Address20) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[12..].copy_from_slice(a);
    w
}

/// One payout as the vault takes it: `amount` in WAD, as Zyn holds it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Payout {
    pub to: Address20,
    pub asset: u32,
    pub amount: u128,
}

/// The vault's EIP-712 domain separator.
pub fn domain_separator(chain_id: u64, vault: &Address20) -> [u8; 32] {
    let t = keccak(&[b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"]);
    keccak(&[&t, &keccak(&[b"ZynVault"]), &keccak(&[b"1"]), &word_u(chain_id as u128), &word_addr(vault)])
}

/// What `ZynVault.withdrawalDigest(nonce, epoch, payouts)` returns.
pub fn withdrawal_digest(chain_id: u64, vault: &Address20, nonce: u64, epoch: u64, payouts: &[Payout]) -> [u8; 32] {
    let pt = h32(PAYOUT_TYPEHASH);
    let hashes: Vec<[u8; 32]> = payouts.iter().map(|p| keccak(&[&pt, &word_addr(&p.to), &word_u(p.asset as u128), &word_u(p.amount)])).collect();
    let packed: Vec<u8> = hashes.concat();
    let s = keccak(&[&h32(WITHDRAWAL_TYPEHASH), &word_u(nonce as u128), &word_u(epoch as u128), &keccak(&[&packed])]);
    keccak(&[&[0x19, 0x01], &domain_separator(chain_id, vault), &s])
}

/// The address a key signs as.
pub fn address_of(key: &SigningKey) -> Address20 {
    let point = key.verifying_key().to_encoded_point(false);
    let h = keccak(&[&point.as_bytes()[1..]]);
    let mut a = [0u8; 20];
    a.copy_from_slice(&h[12..]);
    a
}

/// `(r, s, v)` over `digest` itself, `v` in {27, 28}, low `s`.
pub fn sign(key: &SigningKey, digest: &[u8; 32]) -> Result<[u8; 65], String> {
    let (sig, rec): (Signature, RecoveryId) = key.sign_prehash_recoverable(digest).map_err(|e| e.to_string())?;
    let (sig, rec) = match sig.normalize_s() {
        Some(low) => (low, RecoveryId::from_byte(rec.to_byte() ^ 1).ok_or("recovery id")?),
        None => (sig, rec),
    };
    let mut out = [0u8; 65];
    out[..64].copy_from_slice(&sig.to_bytes());
    out[64] = 27 + rec.to_byte();
    Ok(out)
}

/// What `ZynSigners.isValidSignature` takes: `threshold` signatures,
/// ascending by signer address.
pub fn threshold_signature(keys: &[SigningKey], threshold: usize, digest: &[u8; 32]) -> Result<Vec<u8>, String> {
    if keys.len() < threshold {
        return Err(format!("{} key(s) here, {threshold} needed", keys.len()));
    }
    let mut chosen: Vec<&SigningKey> = keys.iter().collect();
    chosen.sort_by_key(|k| address_of(k));
    let mut out = Vec::with_capacity(threshold * 65);
    for k in chosen.into_iter().take(threshold) {
        out.extend_from_slice(&sign(k, digest)?);
    }
    Ok(out)
}

fn selector(sig: &str) -> [u8; 4] {
    let h = keccak(&[sig.as_bytes()]);
    [h[0], h[1], h[2], h[3]]
}

/// Calldata for `withdraw(uint64 nonce, uint64 epoch, Payout[] payouts, bytes signature)`.
pub fn withdraw_call(nonce: u64, epoch: u64, payouts: &[Payout], signature: &[u8]) -> Vec<u8> {
    let mut out = selector("withdraw(uint64,uint64,(address,uint256,uint256)[],bytes)").to_vec();
    let payouts_len = 32 + payouts.len() * 96;
    out.extend_from_slice(&word_u(nonce as u128));
    out.extend_from_slice(&word_u(epoch as u128));
    out.extend_from_slice(&word_u(128));
    out.extend_from_slice(&word_u((128 + payouts_len) as u128));
    out.extend_from_slice(&word_u(payouts.len() as u128));
    for p in payouts {
        out.extend_from_slice(&word_addr(&p.to));
        out.extend_from_slice(&word_u(p.asset as u128));
        out.extend_from_slice(&word_u(p.amount));
    }
    out.extend_from_slice(&word_u(signature.len() as u128));
    out.extend_from_slice(signature);
    out.resize(out.len() + (32 - signature.len() % 32) % 32, 0);
    out
}

/// Calldata for `withdrawalDigest(uint64, uint64, Payout[])`, to cross-check.
pub fn digest_call(nonce: u64, epoch: u64, payouts: &[Payout]) -> Vec<u8> {
    let mut out = selector("withdrawalDigest(uint64,uint64,(address,uint256,uint256)[])").to_vec();
    out.extend_from_slice(&word_u(nonce as u128));
    out.extend_from_slice(&word_u(epoch as u128));
    out.extend_from_slice(&word_u(96));
    out.extend_from_slice(&word_u(payouts.len() as u128));
    for p in payouts {
        out.extend_from_slice(&word_addr(&p.to));
        out.extend_from_slice(&word_u(p.asset as u128));
        out.extend_from_slice(&word_u(p.amount));
    }
    out
}

/// Calldata for `withdrawAsSigner(uint64 nonce, uint64 epoch, Payout[] payouts)`:
/// the payout a Safe that is the vault's signer executes itself (§155).
pub fn withdraw_as_signer_call(nonce: u64, epoch: u64, payouts: &[Payout]) -> Vec<u8> {
    let mut out = digest_call(nonce, epoch, payouts);
    out[..4].copy_from_slice(&selector("withdrawAsSigner(uint64,uint64,(address,uint256,uint256)[])"));
    out
}

/// Calldata for a Safe's `getTransactionHash(...)` for a plain CALL with no
/// gas refund, so the hash Safe{Wallet} will show is the one the chain computes.
pub fn safe_tx_hash_call(to: &Address20, value: u128, data: &[u8], safe_nonce: u64) -> Vec<u8> {
    let mut out = selector("getTransactionHash(address,uint256,bytes,uint8,uint256,uint256,uint256,address,address,uint256)").to_vec();
    out.extend_from_slice(&word_addr(to));
    out.extend_from_slice(&word_u(value));
    out.extend_from_slice(&word_u(32 * 10)); // offset of `data`
    for _ in 0..6 {
        out.extend_from_slice(&[0u8; 32]); // operation, safeTxGas, baseGas, gasPrice, gasToken, refundReceiver
    }
    out.extend_from_slice(&word_u(safe_nonce as u128));
    out.extend_from_slice(&word_u(data.len() as u128));
    out.extend_from_slice(data);
    out.resize(out.len() + (32 - data.len() % 32) % 32, 0);
    out
}

/// Calldata for `depositNative(bytes32 commitment, uint32 asset)`.
pub fn deposit_native_call(commitment: &[u8; 32], asset: u32) -> Vec<u8> {
    let mut out = selector("depositNative(bytes32,uint32)").to_vec();
    out.extend_from_slice(commitment);
    out.extend_from_slice(&word_u(asset as u128));
    out
}

pub fn nonce_call() -> Vec<u8> {
    selector("nonce()").to_vec()
}

/// `topic0` of `Deposited(bytes32 indexed, uint32 indexed, uint256, uint64)`.
pub fn deposited_topic() -> [u8; 32] {
    keccak(&[b"Deposited(bytes32,uint32,uint256,uint64)"])
}

/// One `Deposited` log: `(commitment, asset, wad, index)`.
pub fn decode_deposited(topics: &[[u8; 32]], data: &[u8]) -> Option<([u8; 32], u32, u128, u64)> {
    if topics.len() != 3 || topics[0] != deposited_topic() || data.len() != 64 {
        return None;
    }
    let asset = u32::from_be_bytes(topics[2][28..].try_into().ok()?);
    if topics[2][..28].iter().any(|b| *b != 0) || data[..16].iter().any(|b| *b != 0) || data[32..56].iter().any(|b| *b != 0) {
        return None;
    }
    let wad = u128::from_be_bytes(data[16..32].try_into().ok()?);
    let index = u64::from_be_bytes(data[56..64].try_into().ok()?);
    Some((topics[1], asset, wad, index))
}

/// The ledger's reference for one vault deposit: the chain, the vault, the
/// asset and the vault's own per-asset index — gap-checkable and public.
pub fn deposit_ref(chain_id: u64, vault: &Address20, asset: u32, index: u64) -> Hash {
    let mut e = Encoder::new();
    e.bytes(b"objectvm.evm-deposit.v1").u64(chain_id).bytes(vault).u32(asset).u64(index);
    e.leaf()
}

// ---------------------------------------------------------------------------
// A legacy (EIP-155) transaction: enough for the bridge to call its vault.

fn rlp_bytes(b: &[u8]) -> Vec<u8> {
    if b.len() == 1 && b[0] < 0x80 {
        return b.to_vec();
    }
    let mut out = rlp_len(b.len(), 0x80);
    out.extend_from_slice(b);
    out
}

fn rlp_len(n: usize, base: u8) -> Vec<u8> {
    if n < 56 {
        return vec![base + n as u8];
    }
    let be: Vec<u8> = (n as u64).to_be_bytes().iter().copied().skip_while(|x| *x == 0).collect();
    let mut out = vec![base + 55 + be.len() as u8];
    out.extend(be);
    out
}

fn rlp_uint(v: u128) -> Vec<u8> {
    let be: Vec<u8> = v.to_be_bytes().iter().copied().skip_while(|x| *x == 0).collect();
    rlp_bytes(&be)
}

fn rlp_list(items: &[Vec<u8>]) -> Vec<u8> {
    let body: Vec<u8> = items.concat();
    let mut out = rlp_len(body.len(), 0xc0);
    out.extend(body);
    out
}

/// A call to send.
pub struct Tx {
    pub nonce: u64,
    pub gas_price: u128,
    pub gas: u64,
    pub to: Address20,
    pub value: u128,
    pub data: Vec<u8>,
    pub chain_id: u64,
}

impl Tx {
    /// Signed and RLP-encoded, ready for `eth_sendRawTransaction`.
    pub fn signed(&self, key: &SigningKey) -> Result<Vec<u8>, String> {
        let base = |extra: Vec<Vec<u8>>| {
            let mut v = vec![rlp_uint(self.nonce as u128), rlp_uint(self.gas_price), rlp_uint(self.gas as u128), rlp_bytes(&self.to), rlp_uint(self.value), rlp_bytes(&self.data)];
            v.extend(extra);
            rlp_list(&v)
        };
        let hash = keccak(&[&base(vec![rlp_uint(self.chain_id as u128), rlp_uint(0), rlp_uint(0)])]);
        let sig = sign(key, &hash)?;
        let v = (sig[64] - 27) as u128 + 35 + 2 * self.chain_id as u128;
        let strip = |b: &[u8]| b.iter().copied().skip_while(|x| *x == 0).collect::<Vec<u8>>();
        Ok(base(vec![rlp_uint(v), rlp_bytes(&strip(&sig[..32])), rlp_bytes(&strip(&sig[32..64]))]))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_safe_path_selectors_are_the_ones_cast_computes() {
        let p = [Payout { to: [0x11; 20], asset: 1, amount: 5 }];
        assert_eq!(&withdraw_as_signer_call(3, 9, &p)[..4], &[0xe6, 0x6d, 0x58, 0x31]);
        // the rest is the digest call's encoding, which the contract already cross-checks
        assert_eq!(&withdraw_as_signer_call(3, 9, &p)[4..], &digest_call(3, 9, &p)[4..]);
        let c = safe_tx_hash_call(&[0x22; 20], 0, &[1, 2, 3], 7);
        assert_eq!(&c[..4], &[0xd8, 0xd1, 0x1f, 0x78]);
        // 10 head words, the length word, one padded data word
        assert_eq!(c.len(), 4 + 32 * 12);
        assert_eq!(c[4 + 32 * 9 + 31], 7);
        assert_eq!(&c[4 + 32 * 11..4 + 32 * 11 + 3], &[1, 2, 3]);
    }

    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    /// The example transaction in EIP-155 itself.
    #[test]
    fn a_signed_transaction_is_eip_155s_own_example() {
        let key = SigningKey::from_bytes(&[0x46; 32].into()).unwrap();
        let tx = Tx { nonce: 9, gas_price: 20_000_000_000, gas: 21_000, to: [0x35; 20], value: 1_000_000_000_000_000_000, data: vec![], chain_id: 1 };
        assert_eq!(
            hex(&tx.signed(&key).unwrap()),
            "f86c098504a817c800825208943535353535353535353535353535353535353535880de0b6b3a76400008025a028ef61340bd939bc2195fe537567866003e1a15d3c71ff63e1590620aa636276a067cbe9d8997f761aecb703304b3800ccf555c9f3dc64214b297fb1966a3b6d83"
        );
    }

    #[test]
    fn the_typehashes_are_the_contracts() {
        assert_eq!(hex(&keccak(&[b"Payout(address to,uint256 asset,uint256 amount)"])), PAYOUT_TYPEHASH);
        assert_eq!(
            hex(&keccak(&[b"Withdrawal(uint256 nonce,uint256 epoch,Payout[] payouts)Payout(address to,uint256 asset,uint256 amount)"])),
            WITHDRAWAL_TYPEHASH
        );
    }

    #[test]
    fn two_signatures_come_ascending_and_recover_to_their_owners() {
        let keys: Vec<SigningKey> = [0xA1u8, 0xB2, 0xC3].iter().map(|b| SigningKey::from_bytes(&[*b; 32].into()).unwrap()).collect();
        let d = [7u8; 32];
        let sig = threshold_signature(&keys, 2, &d).unwrap();
        assert_eq!(sig.len(), 130);
        let rec = |s: &[u8]| {
            let sig = Signature::from_slice(&s[..64]).unwrap();
            let vk = k256::ecdsa::VerifyingKey::recover_from_prehash(&d, &sig, RecoveryId::from_byte(s[64] - 27).unwrap()).unwrap();
            let p = vk.to_encoded_point(false);
            let h = keccak(&[&p.as_bytes()[1..]]);
            <[u8; 20]>::try_from(&h[12..]).unwrap()
        };
        assert!(rec(&sig[..65]) < rec(&sig[65..]), "ascending by owner");
    }

    #[test]
    fn a_deposit_log_reads_back() {
        let mut asset = [0u8; 32];
        asset[31] = 1;
        let mut data = vec![0u8; 64];
        data[16..32].copy_from_slice(&5u128.pow(18).to_be_bytes());
        data[63] = 4;
        assert_eq!(decode_deposited(&[deposited_topic(), [9; 32], asset], &data), Some(([9; 32], 1, 5u128.pow(18), 4)));
        assert_eq!(decode_deposited(&[[0; 32], [9; 32], asset], &data), None);
    }
}
