//! Keys, proving and verifying, in arkworks' own terms.
//!
//! Testnet keys come from a one-party setup with a stated seed ([`setup`]):
//! whoever knows the seed can forge proofs, so they are for testnet only and
//! say so (§158.8). Mainnet keys come from a ceremony.

use crate::circuit::Spend;
use ark_bn254::{Bn254, Fr};
use ark_groth16::{Groth16, Proof, ProvingKey, VerifyingKey};
use ark_snark::SNARK;
use rand_chacha::rand_core::SeedableRng;
use rand_chacha::ChaCha20Rng;

/// Keys for the spend circuit from a single party's `seed`. TESTNET ONLY.
pub fn setup(seed: [u8; 32]) -> (ProvingKey<Bn254>, VerifyingKey<Bn254>) {
    let mut rng = ChaCha20Rng::from_seed(seed);
    Groth16::<Bn254>::circuit_specific_setup(Spend::default(), &mut rng).expect("the circuit is well formed")
}

/// A proof for `spend`, with randomness from `seed` (a wallet passes fresh
/// randomness; tests pass a fixed seed).
pub fn prove(pk: &ProvingKey<Bn254>, spend: Spend, seed: [u8; 32]) -> Result<Proof<Bn254>, String> {
    let mut rng = ChaCha20Rng::from_seed(seed);
    Groth16::<Bn254>::prove(pk, spend, &mut rng).map_err(|e| e.to_string())
}

pub fn verify(vk: &VerifyingKey<Bn254>, public: &[Fr; 3], proof: &Proof<Bn254>) -> bool {
    Groth16::<Bn254>::verify(vk, public, proof).unwrap_or(false)
}
