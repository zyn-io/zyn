//! Deposit receipts (Zyn Lend): a record, as an NFT on the chain the deposit
//! came from, that a deposit into Zyn happened. A record, not a claim — it
//! owns nothing on Zyn, and it cannot be transferred.
//!
//! - **Transparent**: to the address that deposited, showing the asset and
//!   the amount — no more than that chain already shows about the deposit.
//! - **Private**: to any address the depositor names, showing only a hash
//!   that commits to the deposit's pool note with a salt only the depositor's
//!   Zyn account can derive. The depositor's wallet signs the request, so the
//!   receipt service — and nobody else — sees which deposit it records.
//!
//! Ethereum: `contracts/src/receipt/ZynReceipt.sol`. Solana: a Metaplex Core
//! asset, frozen for good (PermanentFreezeDelegate), its details as
//! Attributes; built here byte for byte as the Metaplex client builds it.

use crate::eth_bridge::Address20;
use crate::sol_pool::{Ix, Meta};
use zyn_vm::eip712::keccak;

pub type Pubkey = [u8; 32];

/// Metaplex Core (devnet and mainnet).
pub const MPL_CORE: Pubkey = [
    0xaf, 0x54, 0xab, 0x10, 0xbd, 0x97, 0xa5, 0x42, 0xa0, 0x9e, 0xf7, 0xb3, 0x98, 0x89, 0xdd, 0x0c, 0xd3, 0x94, 0xa4, 0xcc, 0xe9, 0xdf, 0xa6, 0xcd, 0xc9, 0x7e, 0xbe, 0x2d, 0x23, 0x5b, 0xa7, 0x48,
];
const SYSTEM: Pubkey = [0u8; 32];

fn borsh_str(s: &str, out: &mut Vec<u8>) {
    out.extend_from_slice(&(s.len() as u32).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

/// `CreateV1`: a new asset owned by `owner`, its update authority (and the
/// payer, and the signing authority) `authority`, frozen for good, with
/// `attributes`.
pub fn core_create(asset: &Pubkey, authority: &Pubkey, owner: &Pubkey, name: &str, uri: &str, attributes: &[(&str, &str)]) -> Ix {
    let mut data = vec![0u8, 0u8]; // CreateV1, DataState::AccountState
    borsh_str(name, &mut data);
    borsh_str(uri, &mut data);
    data.push(1); // Some(plugins)
    data.extend_from_slice(&2u32.to_le_bytes());
    // PermanentFreezeDelegate { frozen: true }, authority UpdateAuthority
    data.extend_from_slice(&[5, 1, 1, 2]);
    // Attributes { list }, authority UpdateAuthority
    data.push(6);
    data.extend_from_slice(&(attributes.len() as u32).to_le_bytes());
    for (k, v) in attributes {
        borsh_str(k, &mut data);
        borsh_str(v, &mut data);
    }
    data.extend_from_slice(&[1, 2]);
    let m = |key: Pubkey, signer: bool, writable: bool| Meta { key, signer, writable };
    Ix {
        program: MPL_CORE,
        accounts: vec![
            m(*asset, true, true),
            m(MPL_CORE, false, false), // no collection
            m(*authority, true, false),
            m(*authority, true, true), // payer
            m(*owner, false, false),
            m(*authority, false, false), // update authority
            m(SYSTEM, false, false),
            m(MPL_CORE, false, false), // no log wrapper
        ],
        data,
    }
}

/// A transaction signed by several keys, in the order the message lists
/// them (the payer first).
pub fn signed(message: &[u8], signatures: &[[u8; 64]]) -> Vec<u8> {
    let mut out = vec![signatures.len() as u8];
    for s in signatures {
        out.extend_from_slice(s);
    }
    out.extend_from_slice(message);
    out
}

fn selector(sig: &str) -> [u8; 4] {
    let h = keccak(&[sig.as_bytes()]);
    [h[0], h[1], h[2], h[3]]
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

fn abi_string(s: &str) -> Vec<u8> {
    let mut out = word_u(s.len() as u128).to_vec();
    out.extend_from_slice(s.as_bytes());
    out.resize(32 + s.len().div_ceil(32) * 32, 0);
    out
}

/// `mintTransparent(address,bytes32,string,string,uint256,uint8)`.
pub fn mint_transparent_call(to: &Address20, deposit_tx: &[u8; 32], asset: &str, chain_name: &str, amount: u128, decimals: u8) -> Vec<u8> {
    let mut out = selector("mintTransparent(address,bytes32,string,string,uint256,uint8)").to_vec();
    let a = abi_string(asset);
    out.extend_from_slice(&word_addr(to));
    out.extend_from_slice(deposit_tx);
    out.extend_from_slice(&word_u(6 * 32));
    out.extend_from_slice(&word_u((6 * 32 + a.len()) as u128));
    out.extend_from_slice(&word_u(amount));
    out.extend_from_slice(&word_u(decimals as u128));
    out.extend_from_slice(&a);
    out.extend_from_slice(&abi_string(chain_name));
    out
}

/// `mintPrivate(address,bytes32,string)`.
pub fn mint_private_call(to: &Address20, hash: &[u8; 32], chain_name: &str) -> Vec<u8> {
    let mut out = selector("mintPrivate(address,bytes32,string)").to_vec();
    out.extend_from_slice(&word_addr(to));
    out.extend_from_slice(hash);
    out.extend_from_slice(&word_u(3 * 32));
    out.extend_from_slice(&abi_string(chain_name));
    out
}

/// What the depositor's wallet signs to ask for a private receipt.
pub fn private_request_text(chain: &str, deposit_tx: &str, to: &str, hash: &[u8; 32]) -> String {
    let h: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    format!("Zyn private receipt\n\nChain: {chain}\nDeposit: {deposit_tx}\nReceipt to: {to}\nHash: {h}\n\nThis asks for a receipt; it moves nothing.")
}

/// `amount` with `decimals` places, trailing zeros dropped: "0.5".
pub fn amount_text(amount: u128, decimals: u8) -> String {
    let unit = 10u128.pow(decimals as u32);
    let (whole, frac) = (amount / unit, amount % unit);
    if frac == 0 {
        return whole.to_string();
    }
    let f = format!("{frac:0width$}", width = decimals as usize);
    format!("{whole}.{}", f.trim_end_matches('0'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(s: &str) -> Vec<u8> {
        (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect()
    }

    /// The bytes @metaplex-foundation/mpl-core's `createV1` builds for the same
    /// asset (frozen, with two attributes), and its account order.
    #[test]
    fn a_core_asset_is_created_as_the_metaplex_client_creates_it() {
        let ix = core_create(&[1; 32], &[2; 32], &[3; 32], "Zyn deposit: 0.5 SOL", "https://lend.zyn-network.com/receipt/transparent.json", &[("Channel", "Transparent"), ("Amount", "0.5")]);
        assert_eq!(ix.data, h("0000140000005a796e206465706f7369743a20302e3520534f4c3500000068747470733a2f2f6c656e642e7a796e2d6e6574776f726b2e636f6d2f726563656970742f7472616e73706172656e742e6a736f6e0102000000050101020602000000070000004368616e6e656c0b0000005472616e73706172656e7406000000416d6f756e7403000000302e350102"));
        let flags: Vec<(bool, bool)> = ix.accounts.iter().map(|m| (m.signer, m.writable)).collect();
        assert_eq!(flags, vec![(true, true), (false, false), (true, false), (true, true), (false, false), (false, false), (false, false), (false, false)]);
        assert_eq!(zyn_custody::solana::base58_encode(&MPL_CORE), "CoREENxT6tW1HoK8ypY1SxRMZTcVPm7R94rH4PZNhX7d");
    }

    #[test]
    fn the_contract_calls_are_the_ones_cast_builds() {
        // cast sig "mintTransparent(address,bytes32,string,string,uint256,uint8)" / "mintPrivate(address,bytes32,string)"
        let t = mint_transparent_call(&[0x11; 20], &[0x22; 32], "ETH", "Sepolia", 5 * 10u128.pow(17), 18);
        assert_eq!(t.len(), 4 + 32 * 6 + 64 + 64);
        assert_eq!(t[4 + 32 * 2 + 31], 0xc0, "the first string sits after six head words");
        assert_eq!(t[4 + 32 * 3 + 30..4 + 32 * 4], [1, 0], "the second after the first (0x100)");
        let p = mint_private_call(&[0x11; 20], &[0x33; 32], "Sepolia");
        assert_eq!(p.len(), 4 + 32 * 3 + 64);
        assert_eq!(&t[..4], &selector("mintTransparent(address,bytes32,string,string,uint256,uint8)"));
        // cast calldata, same arguments
        assert_eq!(t, h("7b41a7d40000000000000000000000001111111111111111111111111111111111111111222222222222222222222222222222222222222222222222222222222222222200000000000000000000000000000000000000000000000000000000000000c0000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000006f05b59d3b2000000000000000000000000000000000000000000000000000000000000000000120000000000000000000000000000000000000000000000000000000000000003455448000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000075365706f6c696100000000000000000000000000000000000000000000000000"));
        assert_eq!(p, h("8c3c61a900000000000000000000000011111111111111111111111111111111111111113333333333333333333333333333333333333333333333333333333333333333000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000000075365706f6c696100000000000000000000000000000000000000000000000000"));
    }

    #[test]
    fn amounts_read_as_people_write_them() {
        assert_eq!(amount_text(500_000_000, 9), "0.5");
        assert_eq!(amount_text(10u128.pow(18), 18), "1");
        assert_eq!(amount_text(9_950_000_000, 9), "9.95");
    }
}

