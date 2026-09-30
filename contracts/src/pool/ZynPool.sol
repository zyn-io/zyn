// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {PoseidonT3} from "./PoseidonT3.sol";
import {ZynPoolVerifier} from "./ZynPoolVerifier.sol";

/// @title ZynPool — the private bridge channel's pool for one size of the
///        chain's native coin (DECISIONS §158): ETH on Ethereum, BTC on Mezo.
///
/// The same rules as the Solana program (`solana/zyn-pool`), and the same
/// proofs byte for byte:
///
/// - `deposit`: anyone puts in one note of this pool's size.
/// - `spendToZyn`: a proof that the caller owns *one of* the notes, bound to
///   a Zyn deposit code. The coin stays; the bridge credits it on Zyn.
/// - `spendToAddress`: the same proof, bound to a recipient, a relayer and
///   the relayer's fee. The pool pays out.
/// - `insertExit`: the authority adds a note for a private withdrawal from
///   Zyn, only while the pool holds enough for every note outstanding.
/// - `setAuthority`: the authority hands itself on (testnet starts with the
///   bridge's key; the custodians take over — §158.4).
///
/// The note tree is Poseidon, depth 20, zero-filled, with the last 30 roots
/// kept so a proof made a moment ago still lands.
contract ZynPool is ZynPoolVerifier {
    uint256 public constant DEPTH = 20;
    uint256 public constant ROOTS = 30;
    uint256 internal constant FIELD = 21888242871839275222246405745257275088548364400416034343698204186575808495617;

    uint256 public immutable denomination;
    address public authority;
    uint32 public nextIndex;
    uint32 public rootAt;
    /// Notes in the tree not yet spent: each must stay payable.
    uint256 public live;
    uint256[DEPTH] public filled;
    uint256[DEPTH + 1] public zeros;
    uint256[ROOTS] public roots;
    mapping(uint256 => bool) public spent;

    event Deposit(uint256 indexed commitment, uint32 index, uint256 root);
    event Exit(uint256 indexed commitment, uint32 index, uint256 root);
    event ToZyn(uint256 indexed nullifierHash, bytes32 code);
    event ToAddress(uint256 indexed nullifierHash);
    event Authority(address next);

    error NotCanonical();
    error WrongAmount();
    error UnknownRoot();
    error AlreadySpent();
    error BadProof();
    error NotAuthority();
    error Underfunded();
    error FeeTooHigh();
    error TreeFull();
    error PayFailed();

    constructor(uint256 denomination_, address authority_) {
        require(denomination_ > 0, "size");
        denomination = denomination_;
        authority = authority_;
        uint256 z = 0;
        for (uint256 i = 0; i <= DEPTH; i++) {
            zeros[i] = z;
            if (i < DEPTH) {
                filled[i] = z;
                z = PoseidonT3.hash(z, z);
            }
        }
        roots[0] = zeros[DEPTH];
    }

    function _insert(uint256 leaf) internal returns (uint32 index, uint256 root) {
        index = nextIndex;
        if (index >= 2 ** DEPTH) revert TreeFull();
        uint256 at = index;
        uint256 cur = leaf;
        for (uint256 i = 0; i < DEPTH; i++) {
            if (at % 2 == 0) {
                filled[i] = cur;
                cur = PoseidonT3.hash(cur, zeros[i]);
            } else {
                cur = PoseidonT3.hash(filled[i], cur);
            }
            at /= 2;
        }
        uint32 slot = uint32((uint256(rootAt) + 1) % ROOTS);
        rootAt = slot;
        roots[slot] = cur;
        nextIndex = index + 1;
        root = cur;
    }

    function isKnownRoot(uint256 root) public view returns (bool) {
        if (root == 0) return false;
        for (uint256 i = 0; i < ROOTS; i++) {
            if (roots[i] == root) return true;
        }
        return false;
    }

    /// keccak256(domain ‖ data) with its top byte cleared — `zyn_pool::binding`.
    function _binding(bytes memory domain, bytes memory data) internal pure returns (uint256) {
        return uint256(keccak256(bytes.concat(domain, data))) & ((1 << 248) - 1);
    }

    function deposit(uint256 commitment) external payable {
        if (commitment >= FIELD) revert NotCanonical();
        if (msg.value != denomination) revert WrongAmount();
        (uint32 index, uint256 root) = _insert(commitment);
        live += 1;
        emit Deposit(commitment, index, root);
    }

    function _spend(bytes calldata proof, uint256 root, uint256 nullifierHash, uint256 binding) internal {
        if (root >= FIELD || nullifierHash >= FIELD) revert NotCanonical();
        if (!isKnownRoot(root)) revert UnknownRoot();
        if (spent[nullifierHash]) revert AlreadySpent();
        if (!verifyProof(proof, [root, nullifierHash, binding])) revert BadProof();
        spent[nullifierHash] = true;
        live -= 1;
    }

    /// Into Zyn under `code`. The coin stays: it now backs the bridged asset.
    function spendToZyn(bytes calldata proof, uint256 root, uint256 nullifierHash, bytes32 code) external {
        _spend(proof, root, nullifierHash, _binding("zyn-pool.to-zyn.v1", abi.encodePacked(code)));
        emit ToZyn(nullifierHash, code);
    }

    /// Out to `recipient`, `fee` of it to `relayer` (who submits it, so the
    /// recipient needs nothing to begin with).
    function spendToAddress(bytes calldata proof, uint256 root, uint256 nullifierHash, address payable recipient, address payable relayer, uint256 fee) external {
        if (fee > denomination) revert FeeTooHigh();
        bytes memory data = abi.encodePacked(uint16(20), recipient, relayer, uint64(fee));
        _spend(proof, root, nullifierHash, _binding("zyn-pool.to-address.v1", data));
        emit ToAddress(nullifierHash);
        (bool ok,) = recipient.call{value: denomination - fee}("");
        if (!ok) revert PayFailed();
        if (fee > 0) {
            (ok,) = relayer.call{value: fee}("");
            if (!ok) revert PayFailed();
        }
    }

    /// A private withdrawal's note, from the authority, while every note in
    /// the tree (and this one) stays payable.
    function insertExit(uint256 commitment) external {
        if (msg.sender != authority) revert NotAuthority();
        if (commitment >= FIELD) revert NotCanonical();
        if (address(this).balance < denomination * (live + 1)) revert Underfunded();
        (uint32 index, uint256 root) = _insert(commitment);
        live += 1;
        emit Exit(commitment, index, root);
    }

    function setAuthority(address next) external {
        if (msg.sender != authority) revert NotAuthority();
        authority = next;
        emit Authority(next);
    }

    /// The bridge tops the pool up from its vault when an exit needs backing.
    receive() external payable {}
}
