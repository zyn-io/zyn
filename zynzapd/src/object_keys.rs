//! The wallet's identity on the object ledger.
//!
//! Nap already derives an identity and receiving accounts from one seed:
//!
//! ```text
//!   wallet identity   HKDF(seed, "nap.zyn.ed25519.v1", "account" || account)
//!   receiving account HKDF(seed, "nap.zyn.receive.v1", "receive" || account || index)
//! ```
//!
//! The object ledger needs two more, and they are **not** the identity key.
//! One finds notes, one recognises spends, and neither can move anything:
//!
//! ```text
//!   viewing key       HKDF(seed, "nap.zyn.viewing.v1",   "viewing"   || account)
//!   nullifier key     HKDF(seed, "nap.zyn.nullifier.v1", "nullifier" || account)
//! ```
//!
//! # Why three keys and not one
//!
//! Because they can be handed out separately, and separating them is the only
//! reason a wallet can be watched without being spendable:
//!
//! | key | can | cannot |
//! |---|---|---|
//! | identity | authorise a spend | — |
//! | viewing | see what arrived | see what left, spend anything |
//! | nullifier | recognise what left | spend anything |
//!
//! Give an auditor the viewing key and they see receipts. Give them both
//! viewing and nullifier keys and they see a correct balance. Neither lets
//! them move a coin, and the identity key never leaves.
//!
//! Deriving them from the same seed is what keeps the recovery promise: one
//! phrase restores the wallet, everything it was paid, and everything it
//! spent. A viewing key generated independently would be a second thing to
//! back up, and a second thing to back up is a thing somebody loses.
//!
//! # These are a wire format
//!
//! An account already registered on chain is bound to the viewing key these
//! produce (§register_viewing_key). Change a salt and every restored wallet
//! looks at the wrong ciphertexts, finds nothing, and reports zero — which is
//! indistinguishable from having been robbed. The salts are pinned by a test.

use hkdf::Hkdf;
use sha2::Sha256;

use zyn_vm::note_enc::{viewing_public, StaticSecret, ViewingKey};
use zyn_vm::object::NullifierKey;

/// Finds notes. Never spends them.
const VIEWING_SALT: &[u8] = b"nap.zyn.viewing.v1";
const VIEWING_INFO: &[u8] = b"viewing";

/// Recognises spends. Also never spends them — it only computes the
/// nullifiers a spend *would* publish, so a scanner can match them.
const NULLIFIER_SALT: &[u8] = b"nap.zyn.nullifier.v1";
const NULLIFIER_INFO: &[u8] = b"nullifier";

fn derive(seed: &[u8], salt: &[u8], info: &[u8], account: u32) -> [u8; 32] {
    let mut labelled = info.to_vec();
    labelled.extend_from_slice(&account.to_be_bytes());
    let mut out = [0u8; 32];
    // Expanding into a fixed 32 bytes cannot fail at this length; the `expect`
    // documents that rather than hiding a decision.
    Hkdf::<Sha256>::new(Some(salt), seed)
        .expand(&labelled, &mut out)
        .expect("32 bytes is within HKDF's output limit");
    out
}

/// The key that opens notes sent to this account.
pub fn viewing_secret(seed: &[u8], account: u32) -> StaticSecret {
    StaticSecret::from(derive(seed, VIEWING_SALT, VIEWING_INFO, account))
}

/// What to publish so others can pay this account. This is the value that
/// goes on chain in a registration, and the one a payer encrypts to.
pub fn viewing_key(seed: &[u8], account: u32) -> ViewingKey {
    viewing_public(&viewing_secret(seed, account))
}

/// The key that recognises this account's own spends.
pub fn nullifier_key(seed: &[u8], account: u32) -> NullifierKey {
    derive(seed, NULLIFIER_SALT, NULLIFIER_INFO, account)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: &[u8] = b"a seed that is not a real seed";

    /// The recovery promise: one phrase, the same wallet, every time.
    #[test]
    fn the_same_seed_and_account_give_the_same_keys() {
        assert_eq!(viewing_key(SEED, 0), viewing_key(SEED, 0));
        assert_eq!(nullifier_key(SEED, 0), nullifier_key(SEED, 0));
    }

    /// Two accounts under one seed must not share a viewing key, or a payer
    /// encrypting to one would produce notes the other can open.
    #[test]
    fn accounts_do_not_share_keys() {
        assert_ne!(viewing_key(SEED, 0), viewing_key(SEED, 1));
        assert_ne!(nullifier_key(SEED, 0), nullifier_key(SEED, 1));
    }

    /// The whole reason there are two: a viewing key must not be enough to
    /// derive the nullifier key. If it were, handing an auditor read access
    /// would hand them the ability to link every spend.
    #[test]
    fn the_viewing_key_does_not_reveal_the_nullifier_key() {
        let v = viewing_key(SEED, 0);
        let n = nullifier_key(SEED, 0);
        assert_ne!(v, n);
        // Different salts, so neither is a relabelling of the other.
        assert_ne!(
            derive(SEED, VIEWING_SALT, VIEWING_INFO, 0),
            derive(SEED, NULLIFIER_SALT, NULLIFIER_INFO, 0)
        );
    }

    /// Nap's existing derivations must not collide with these, or an identity
    /// key and a viewing key could be the same bytes wearing two hats.
    #[test]
    fn the_object_keys_are_distinct_from_naps_existing_ones() {
        let identity = derive(SEED, b"nap.zyn.ed25519.v1", b"account", 0);
        let receive = derive(SEED, b"nap.zyn.receive.v1", b"receive", 0);
        let v = derive(SEED, VIEWING_SALT, VIEWING_INFO, 0);
        let n = derive(SEED, NULLIFIER_SALT, NULLIFIER_INFO, 0);
        for a in [identity, receive] {
            assert_ne!(a, v);
            assert_ne!(a, n);
        }
    }

    /// The salts are a wire format. An account registered on chain is bound
    /// to the viewing key this produces; change a salt and every restored
    /// wallet looks at the wrong ciphertexts, finds nothing, and reports zero
    /// — which looks exactly like having been robbed.
    #[test]
    fn the_salts_are_pinned() {
        assert_eq!(VIEWING_SALT, b"nap.zyn.viewing.v1");
        assert_eq!(NULLIFIER_SALT, b"nap.zyn.nullifier.v1");
        assert_eq!(VIEWING_INFO, b"viewing");
        assert_eq!(NULLIFIER_INFO, b"nullifier");
    }
}
