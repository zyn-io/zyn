//! Threshold-controlled Zyn accounts, provisioned in one call.
//!
//! A treasury is an account no single person can spend from: `t` of `n`
//! holders must agree. Zyn already had every piece needed for one — the DKG is
//! generic over ciphersuite, `frost-ed25519` was already a dependency, and the
//! VM accepts an ordinary Ed25519 signature. What it lacked was a name for the
//! combination, so provisioning one looked like new cryptography when it was
//! assembly.
//!
//! # Why no VM change is needed
//!
//! A FROST group over Ed25519 produces an **ordinary** Ed25519 signature
//! against an **ordinary** Ed25519 verifying key. The chain cannot tell — and
//! does not need to tell — whether one key signed or five cooperated. So a
//! treasury is a normal [`Scheme::Ed25519`] account, and every existing
//! authorisation path works unchanged.
//!
//! # Using it for the next project
//!
//! ```ignore
//! let t = Treasury::provision(2, 3)?;   // 2-of-3
//! let account = t.account;              // pin this; it is the identity
//! // distribute t.shares, one per holder, and never keep two together
//! ```
//!
//! The same call serves a protocol treasury, a project's raise, or a grant
//! pool. Only `threshold` and `participants` change.

use std::collections::BTreeMap;

use frost_core::keys::PublicKeyPackage;

use crate::ceremony::{CeremonyError, IdentifierFor, ThresholdKeys};

/// The ciphersuite a threshold-controlled Zyn account uses: plain Ed25519, so
/// the group's signature is indistinguishable from a single signer's.
pub type ZynAccount = frost_ed25519::Ed25519Sha512;

/// A provisioned treasury: the account it speaks for, and one share per holder.
pub struct Treasury {
    /// The 32-byte Zyn account. Pin this — it is the treasury's identity, and
    /// every address derived from it moves if it changes.
    pub account: [u8; 32],
    /// The group verifying key, as the chain will see it.
    pub group_key: [u8; 32],
    /// One share per holder. **Never store two in one place** — that is the
    /// whole point of the threshold.
    pub shares: BTreeMap<IdentifierFor<ZynAccount>, ThresholdKeys<ZynAccount>>,
}

impl Treasury {
    /// Run the ceremony and return the account plus its shares.
    ///
    /// This drives the DKG in memory, which is right for provisioning where the
    /// participants are machines an operator already controls. A ceremony
    /// across mutually distrusting holders uses the same rounds over the
    /// network — [`crate::dkg_net::DkgParticipant`] — and produces the same
    /// result; this is the reference for it.
    pub fn provision(threshold: u16, participants: u16) -> Result<Treasury, CeremonyError> {
        if threshold < 2 || threshold > participants {
            return Err(CeremonyError::Crypto);
        }
        let shares = crate::dkg_net::run_in_memory_for::<ZynAccount>(threshold, participants)?;
        let public = shares
            .values()
            .next()
            .ok_or(CeremonyError::Crypto)?
            .public_package
            .clone();
        let group_key = group_key_bytes(&public)?;
        Ok(Treasury { account: account_of_group(&public)?, group_key, shares })
    }
}

/// The group's verifying key as the 32 bytes the chain sees.
pub fn group_key_bytes(public: &PublicKeyPackage<ZynAccount>) -> Result<[u8; 32], CeremonyError> {
    public
        .verifying_key()
        .serialize()
        .map_err(|_| CeremonyError::Crypto)?
        .try_into()
        .map_err(|_| CeremonyError::Crypto)
}

/// The Zyn account a threshold group speaks for.
///
/// Identical to what a single Ed25519 key would produce for the same verifying
/// key, which is exactly why the VM needs no threshold-specific path.
pub fn account_of_group(public: &PublicKeyPackage<ZynAccount>) -> Result<[u8; 32], CeremonyError> {
    Ok(zyn_vm::auth::account_of(
        zyn_vm::auth::Scheme::Ed25519,
        &group_key_bytes(public)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provisioning_yields_an_account_and_one_share_per_holder() {
        let t = Treasury::provision(2, 3).expect("provision");
        assert_eq!(t.shares.len(), 3, "one share per holder");
        assert_ne!(t.account, [0u8; 32], "a zero account is the unprovisioned sentinel");
        // Every holder agrees which account they are custodians of.
        for keys in t.shares.values() {
            assert_eq!(account_of_group(&keys.public_package).unwrap(), t.account);
        }
    }

    /// The property that means the chain needs no threshold-aware code: the
    /// account is what a single Ed25519 key with the same verifying key would
    /// produce.
    #[test]
    fn a_treasury_account_is_an_ordinary_ed25519_account() {
        let t = Treasury::provision(2, 3).expect("provision");
        assert_eq!(
            t.account,
            zyn_vm::auth::account_of(zyn_vm::auth::Scheme::Ed25519, &t.group_key),
            "a treasury must be indistinguishable from a single-key account"
        );
    }

    #[test]
    fn two_ceremonies_never_produce_the_same_account() {
        let a = Treasury::provision(2, 3).expect("a");
        let b = Treasury::provision(2, 3).expect("b");
        assert_ne!(a.account, b.account, "each provisioning is a distinct treasury");
    }

    #[test]
    fn a_threshold_below_two_or_above_the_holders_is_refused() {
        assert!(Treasury::provision(1, 3).is_err(), "1-of-n is not a threshold");
        assert!(Treasury::provision(4, 3).is_err(), "more signers than holders can never sign");
    }
}
