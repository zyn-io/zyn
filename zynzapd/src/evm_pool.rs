//! The private channel's EVM pools (DECISIONS §158): `contracts/src/pool/
//! ZynPool.sol`'s calls and logs, in its encoding, byte for byte.
//!
//! One contract per size of the chain's native coin — ETH on Ethereum, BTC
//! on Mezo. The proofs are the Solana program's (`sol_pool`), so everything
//! here is only how the EVM asks for the same thing.

use crate::eth_bridge::Address20;
use zyn_vm::eip712::keccak;

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

pub fn topic(event: &str) -> [u8; 32] {
    keccak(&[event.as_bytes()])
}

pub fn deposit_topic() -> [u8; 32] {
    topic("Deposit(uint256,uint32,uint256)")
}
pub fn exit_topic() -> [u8; 32] {
    topic("Exit(uint256,uint32,uint256)")
}
pub fn to_zyn_topic() -> [u8; 32] {
    topic("ToZyn(uint256,bytes32)")
}
pub fn to_address_topic() -> [u8; 32] {
    topic("ToAddress(uint256)")
}

/// What one pool log says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A note in the tree at `index`, deposited or inserted for an exit.
    Note { commitment: [u8; 32], index: u32, exit: bool },
    /// A spend into Zyn under `code`.
    ToZyn { nullifier_hash: [u8; 32], code: [u8; 32] },
    /// A spend to an address.
    ToAddress { nullifier_hash: [u8; 32] },
}

pub fn decode(topics: &[[u8; 32]], data: &[u8]) -> Option<Event> {
    let t0 = *topics.first()?;
    if (t0 == deposit_topic() || t0 == exit_topic()) && topics.len() == 2 && data.len() == 64 {
        if data[..28].iter().any(|b| *b != 0) {
            return None;
        }
        let index = u32::from_be_bytes(data[28..32].try_into().ok()?);
        return Some(Event::Note { commitment: topics[1], index, exit: t0 == exit_topic() });
    }
    if t0 == to_zyn_topic() && topics.len() == 2 && data.len() == 32 {
        return Some(Event::ToZyn { nullifier_hash: topics[1], code: data.try_into().ok()? });
    }
    if t0 == to_address_topic() && topics.len() == 2 {
        return Some(Event::ToAddress { nullifier_hash: topics[1] });
    }
    None
}

/// The ledger's reference for a spend into Zyn from an EVM pool: the chain,
/// the pool and the nullifier hash — one credit per spent note.
pub fn spend_ref(chain_id: u64, pool: &Address20, nullifier_hash: &[u8; 32]) -> [u8; 32] {
    keccak(&[b"zyn-pool.to-zyn.v1", b"eip155", &chain_id.to_be_bytes(), pool, nullifier_hash])
}

fn with_proof(mut head: Vec<u8>, proof: &[u8; 256]) -> Vec<u8> {
    head.extend_from_slice(&word_u(256));
    head.extend_from_slice(proof);
    head
}

pub fn spend_to_zyn_call(proof: &[u8; 256], root: &[u8; 32], nullifier_hash: &[u8; 32], code: &[u8; 32]) -> Vec<u8> {
    let mut out = selector("spendToZyn(bytes,uint256,uint256,bytes32)").to_vec();
    out.extend_from_slice(&word_u(4 * 32));
    out.extend_from_slice(root);
    out.extend_from_slice(nullifier_hash);
    out.extend_from_slice(code);
    with_proof(out, proof)
}

pub fn spend_to_address_call(proof: &[u8; 256], root: &[u8; 32], nullifier_hash: &[u8; 32], recipient: &Address20, relayer: &Address20, fee: u128) -> Vec<u8> {
    let mut out = selector("spendToAddress(bytes,uint256,uint256,address,address,uint256)").to_vec();
    out.extend_from_slice(&word_u(6 * 32));
    out.extend_from_slice(root);
    out.extend_from_slice(nullifier_hash);
    out.extend_from_slice(&word_addr(recipient));
    out.extend_from_slice(&word_addr(relayer));
    out.extend_from_slice(&word_u(fee));
    with_proof(out, proof)
}

pub fn insert_exit_call(commitment: &[u8; 32]) -> Vec<u8> {
    let mut out = selector("insertExit(uint256)").to_vec();
    out.extend_from_slice(commitment);
    out
}

pub fn sweep_call(to: &Address20, amount: u128) -> Vec<u8> {
    let mut out = selector("sweep(address,uint256)").to_vec();
    out.extend_from_slice(&word_addr(to));
    out.extend_from_slice(&word_u(amount));
    out
}

pub fn spent_call(nullifier_hash: &[u8; 32]) -> Vec<u8> {
    let mut out = selector("spent(uint256)").to_vec();
    out.extend_from_slice(nullifier_hash);
    out
}

pub fn known_root_call(root: &[u8; 32]) -> Vec<u8> {
    let mut out = selector("isKnownRoot(uint256)").to_vec();
    out.extend_from_slice(root);
    out
}

/// The pool's refusal in words, from a revert's data where the chain gives it.
pub fn refusal(error: &str) -> Option<&'static str> {
    [
        ("0xd4d9b0fd", "that note is already spent"),
        ("0x8c520116", "the pool does not know that root; refresh the notes and try again"),
        ("0x7ca55c77", "the proof does not hold for this spend"),
        ("0xcd4e6167", "the fee is more than the note"),
        ("0xd7c7beeb", "a value is not a field element"),
    ]
    .into_iter()
    .find(|(sel, _)| error.contains(sel))
    .map(|(_, why)| why)
}

pub fn live_call() -> Vec<u8> {
    selector("live()").to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(s: &str) -> Vec<u8> {
        (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn selectors_and_topics_are_the_ones_cast_computes() {
        assert_eq!(insert_exit_call(&[0; 32])[..4], h("2c5bbff7")[..]);
        assert_eq!(sweep_call(&[0; 20], 0)[..4], h("6ea056a9")[..]);
        assert_eq!(live_call(), h("957aa58c"));
        assert_eq!(refusal("execution reverted (0xd4d9b0fd)"), Some("that note is already spent"));
        assert_eq!(refusal("execution reverted"), None);
        assert_eq!(to_zyn_topic().to_vec(), h("93b1c6d36782557dfb3df094ba1b9239d5d61fb9d6d15bb36452fade081981f1"));
        assert_eq!(exit_topic().to_vec(), h("05b1cbbc5a122cae42a1d8bed752d3aefbbf73059190cde035f312789e8f4bbf"));
        assert_eq!(to_address_topic().to_vec(), h("c6a7121686640d8369b0e4e740e868f764628f37eb4ba5546d3545d7c0f4f3b1"));
        assert_eq!(deposit_topic().to_vec(), h("2813ca2762c14ad53880ef467c7448a9015904c20e064e6216ffb3f63390ec5d"));
    }

    #[test]
    fn a_spend_into_zyn_is_encoded_as_cast_encodes_it() {
        let mut root = [0u8; 32];
        root[31] = 7;
        let mut nh = [0u8; 32];
        nh[31] = 9;
        let c = spend_to_zyn_call(&[0xab; 256], &root, &nh, &[0x42; 32]);
        // cast calldata "spendToZyn(bytes,uint256,uint256,bytes32)" 0xab…(256) 7 9 0x42…(32)
        assert_eq!(c[..4], h("5f3cd8ec")[..]);
        assert_eq!(c.len(), 4 + 32 * 5 + 256);
        assert_eq!(c[4 + 31], 0x80);
        assert_eq!(c[4 + 32 * 4 + 30..4 + 32 * 4 + 32], [1, 0]);
        assert!(c[4 + 32 * 5..].iter().all(|b| *b == 0xab));
        let a = spend_to_address_call(&[0xab; 256], &root, &nh, &[0x11; 20], &[0x22; 20], 5);
        assert_eq!(a[..4], h("432cd732")[..]);
        assert_eq!(a[4 + 31], 0xc0);
        assert_eq!(a[4 + 32 * 3 + 12..4 + 32 * 4], [0x11; 20]);
        assert_eq!(a[4 + 32 * 6 - 1], 5);
    }

    #[test]
    fn logs_read_back_and_nothing_else_passes_for_one() {
        let mut data = [0u8; 64];
        data[31] = 3;
        let e = decode(&[exit_topic(), [5; 32]], &data).unwrap();
        assert_eq!(e, Event::Note { commitment: [5; 32], index: 3, exit: true });
        assert_eq!(decode(&[to_zyn_topic(), [6; 32]], &[7; 32]), Some(Event::ToZyn { nullifier_hash: [6; 32], code: [7; 32] }));
        assert_eq!(decode(&[to_address_topic(), [6; 32]], &[]), Some(Event::ToAddress { nullifier_hash: [6; 32] }));
        assert_eq!(decode(&[[1; 32], [6; 32]], &[7; 32]), None);
        assert_eq!(decode(&[to_zyn_topic()], &[7; 32]), None);
    }

    #[test]
    fn spend_refs_differ_by_chain_and_pool() {
        let a = spend_ref(11155111, &[1; 20], &[9; 32]);
        assert_ne!(a, spend_ref(31611, &[1; 20], &[9; 32]));
        assert_ne!(a, spend_ref(11155111, &[2; 20], &[9; 32]));
    }
}
