// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ECDSA} from "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";

/// @title ZynSigners — k of n keys, as one EIP-1271 signer.
///
/// @notice The `signer` a `ZynVault` checks withdrawals against (DECISIONS
/// §146). A withdrawal digest is valid when `threshold` distinct owners have
/// each signed it, their signatures concatenated in ascending order of owner
/// address — the order makes "distinct" a comparison instead of a set.
///
/// Fixed at deployment, like the vault's own `signer`: a set that could be
/// changed is a key with the power to change it. Rotating keys is a new
/// signer and a new vault, announced, with the old one drained by withdrawals.
///
/// A Safe does the same job and more; this exists because it is short enough
/// to read in one sitting, which is the property a bridge's signer should have.
contract ZynSigners {
    bytes4 private constant MAGIC = 0x1626ba7e;
    bytes4 private constant INVALID = 0xffffffff;

    uint8 public immutable threshold;
    mapping(address => bool) public isOwner;
    address[] private owners_;

    constructor(address[] memory owners, uint8 threshold_) {
        require(threshold_ > 0 && threshold_ <= owners.length, "threshold");
        for (uint256 i = 0; i < owners.length; i++) {
            address o = owners[i];
            require(o != address(0) && !isOwner[o], "owner");
            isOwner[o] = true;
        }
        owners_ = owners;
        threshold = threshold_;
    }

    function owners() external view returns (address[] memory) {
        return owners_;
    }

    /// EIP-1271. `signatures` is `threshold` 65-byte (r, s, v) signatures over
    /// `hash` itself, ascending by signer.
    function isValidSignature(bytes32 hash, bytes calldata signatures) external view returns (bytes4) {
        if (signatures.length != uint256(threshold) * 65) return INVALID;
        address last = address(0);
        for (uint256 i = 0; i < threshold; i++) {
            bytes calldata one = signatures[i * 65:(i + 1) * 65];
            (address who, ECDSA.RecoverError err,) = ECDSA.tryRecover(hash, one);
            if (err != ECDSA.RecoverError.NoError || !isOwner[who] || who <= last) return INVALID;
            last = who;
        }
        return MAGIC;
    }
}
