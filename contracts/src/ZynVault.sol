// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {IERC20Metadata} from "@openzeppelin/contracts/token/ERC20/extensions/IERC20Metadata.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {SignatureChecker} from "@openzeppelin/contracts/utils/cryptography/SignatureChecker.sol";
import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {Address} from "@openzeppelin/contracts/utils/Address.sol";

/// @title ZynVault — the EVM side of the Zyn bridge.
///
/// @notice Holds value on an EVM chain against units issued on Zyn.
///
/// # Why this is a contract and not an address
///
/// A deposit has to say who it is for. An ERC-20 `transfer` to an externally
/// owned account carries no memo and no calldata, so the only thing a watcher
/// could key on is the sender — which would publish a permanent on-chain link
/// from every depositor's EVM identity to their Zyn account. On a chain whose
/// premise is that trades are shielded, that is the entire leak, at the front
/// door. A contract has calldata, so the destination is an argument.
///
/// Four things follow that an address cannot do at all: refusing an
/// unsupported asset instead of stranding it, measuring what *actually*
/// arrived so fee-on-transfer tokens credit correctly, emitting an ordered
/// index the VM checks for gaps and replays, and pausing.
///
/// # What authorises a withdrawal
///
/// `signer` — either a Safe (verified through EIP-1271) or a threshold-ECDSA
/// address (verified through `ecrecover`). `SignatureChecker` accepts both, so
/// moving from one to the other never changes this contract or its address.
///
/// Signers do not choose withdrawals. The payout list is a deterministic
/// function of Zyn state produced by `zyn_bridge::settle::group`; a signature
/// attests to what the state already determined.
///
/// # Replay is the default failure here
///
/// This is deployed through CREATE2 at the same address on every EVM chain,
/// which is what makes adding a chain a config row rather than an audit. The
/// direct consequence is that a digest not binding the chain authorises the
/// identical withdrawal everywhere. The domain binds `chainId` and
/// `verifyingContract`; the nonce stops a repeat on this chain.
contract ZynVault is ReentrancyGuard {
    using SafeERC20 for IERC20;

    /// Must equal `zyn_bridge::evm`'s encoding exactly. Pinned by a test on the
    /// Rust side (`the_type_strings_are_what_the_contract_expects`), because a
    /// drifted typehash makes every signature fail to verify on a chain holding
    /// funds, with nothing in between to notice.
    ///
    /// keccak256("Withdrawal(uint256 nonce,uint256 epoch,Payout[] payouts)Payout(address to,uint256 asset,uint256 amount)")
    bytes32 public constant WITHDRAWAL_TYPEHASH =
        0x4bbec971ea7bfa995baa1756a56c4ea39f88b2b17e6e7a9813e430ce708de506;
    /// keccak256("Payout(address to,uint256 asset,uint256 amount)")
    bytes32 public constant PAYOUT_TYPEHASH =
        0x3fbf17ffac88b99d8e834e3fbf5378bbf17176b75d0466005ce02d6d0b1a3854;
    bytes32 private constant DOMAIN_TYPEHASH =
        keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)");

    /// Zyn's fixed-point scale. `Fixed` is a WAD-scaled i128, chosen to match
    /// Solidity's convention, so an 18-decimal token needs no rescaling.
    uint256 private constant WAD = 1e18;
    /// A credited amount must be representable as the VM's `Fixed(i128)`.
    uint256 private constant MAX_WAD = uint256(uint128(type(int128).max));

    struct Payout {
        address to;
        uint256 asset;
        uint256 amount; // WAD-scaled, as Zyn holds it
    }

    struct Asset {
        address token; // address(0) => the chain's native asset
        uint8 decimals;
        bool registered;
    }

    /// Authorises withdrawals. Immutable: a mutable one is a second key with
    /// the same power as the first.
    address public immutable signer;

    /// May register assets and pause deposits. Deliberately cannot move funds,
    /// cannot change a registered asset, and **cannot pause withdrawals** —
    /// pausing exits is the power to trap other people's money, and no role
    /// here has it.
    address public admin;

    bool public depositsPaused;
    uint64 public nonce;

    mapping(uint32 => Asset) public assets;
    /// Per asset, matching the VM's `next_deposit_index`.
    mapping(uint32 => uint64) public depositIndex;

    event Deposited(
        bytes32 indexed zynAccount, uint32 indexed asset, uint256 amount, uint64 index
    );
    event Withdrawn(uint64 indexed nonce, uint64 indexed epoch, uint256 count);
    event PaidOut(address indexed to, uint32 indexed asset, uint256 tokenAmount);
    event AssetRegistered(uint32 indexed asset, address token, uint8 decimals);
    event DepositsPaused(bool paused);
    event AdminTransferred(address indexed from, address indexed to);

    error NotAdmin();
    error AlreadyRegistered();
    error UnknownAsset();
    error DepositsArePaused();
    error ZeroAmount();
    error NoDestination();
    error BadSignature();
    error BadNonce();
    error NotRepresentable();
    error InexactScaling();
    error WrongAssetKind();
    error NotSigner();

    modifier onlyAdmin() {
        if (msg.sender != admin) revert NotAdmin();
        _;
    }

    constructor(address signer_, address admin_) {
        require(signer_ != address(0) && admin_ != address(0), "zero");
        signer = signer_;
        admin = admin_;
    }

    // ---------------------------------------------------------------- assets

    /// Add an asset. Add-only on purpose: re-pointing a live asset id at a
    /// different token would let this role rewrite what deposited value *is*,
    /// which is indistinguishable from being able to spend it.
    function registerAsset(uint32 asset, address token) external onlyAdmin {
        if (assets[asset].registered) revert AlreadyRegistered();
        uint8 dec = 18;
        if (token != address(0)) dec = IERC20Metadata(token).decimals();
        assets[asset] = Asset({token: token, decimals: dec, registered: true});
        emit AssetRegistered(asset, token, dec);
    }

    function pauseDeposits(bool paused) external onlyAdmin {
        depositsPaused = paused;
        emit DepositsPaused(paused);
    }

    function transferAdmin(address to) external onlyAdmin {
        require(to != address(0), "zero");
        emit AdminTransferred(admin, to);
        admin = to;
    }

    // -------------------------------------------------------------- deposits

    /// Deposit an ERC-20. Requires prior `approve`.
    ///
    /// `zynAccount` is the destination and need not relate to `msg.sender` —
    /// that freedom is the reason this function exists.
    function deposit(bytes32 zynAccount, uint32 asset, uint256 amount)
        external
        nonReentrant
    {
        Asset memory a = _asset(asset);
        if (a.token == address(0)) revert WrongAssetKind();
        if (zynAccount == bytes32(0)) revert NoDestination();
        if (amount == 0) revert ZeroAmount();
        if (depositsPaused) revert DepositsArePaused();

        // Measure what arrived rather than what was asked for: a
        // fee-on-transfer or rebasing token delivers less, and crediting the
        // requested figure would issue units the vault does not hold.
        uint256 before = IERC20(a.token).balanceOf(address(this));
        IERC20(a.token).safeTransferFrom(msg.sender, address(this), amount);
        uint256 received = IERC20(a.token).balanceOf(address(this)) - before;
        if (received == 0) revert ZeroAmount();

        _credit(zynAccount, asset, _toWad(received, a.decimals));
    }

    /// Deposit the chain's native asset.
    function depositNative(bytes32 zynAccount, uint32 asset)
        external
        payable
        nonReentrant
    {
        Asset memory a = _asset(asset);
        if (a.token != address(0)) revert WrongAssetKind();
        if (zynAccount == bytes32(0)) revert NoDestination();
        if (msg.value == 0) revert ZeroAmount();
        if (depositsPaused) revert DepositsArePaused();
        _credit(zynAccount, asset, _toWad(msg.value, a.decimals));
    }

    function _credit(bytes32 zynAccount, uint32 asset, uint256 wad) private {
        if (wad == 0) revert ZeroAmount();
        if (wad > MAX_WAD) revert NotRepresentable();
        uint64 index = depositIndex[asset];
        depositIndex[asset] = index + 1;
        emit Deposited(zynAccount, asset, wad, index);
    }

    // ----------------------------------------------------------- withdrawals

    /// Pay out one settlement.
    ///
    /// Permissionless to call: the signature is the authority, not the caller.
    /// Anyone holding a valid one may push it through, so a user is never
    /// waiting on a particular operator's willingness to broadcast.
    function withdraw(
        uint64 nonce_,
        uint64 epoch,
        Payout[] calldata payouts,
        bytes calldata signature
    ) external nonReentrant {
        if (nonce_ != nonce) revert BadNonce();

        bytes32 digest = withdrawalDigest(nonce_, epoch, payouts);
        if (!SignatureChecker.isValidSignatureNow(signer, digest, signature)) {
            revert BadSignature();
        }

        _pay(nonce_, epoch, payouts);
    }

    /// Pay out one settlement as the signer itself.
    ///
    /// When the signer is a Safe, its owners can approve the payout as an
    /// ordinary Safe transaction instead of producing an EIP-1271 signature
    /// for someone else to submit. The authority is the same (the signer's
    /// own approval) and so are the checks: the next nonce, the same payout
    /// rules. Only the signer can call it, so it adds no new key.
    function withdrawAsSigner(uint64 nonce_, uint64 epoch, Payout[] calldata payouts)
        external
        nonReentrant
    {
        if (msg.sender != signer) revert NotSigner();
        if (nonce_ != nonce) revert BadNonce();
        _pay(nonce_, epoch, payouts);
    }

    function _pay(uint64 nonce_, uint64 epoch, Payout[] calldata payouts) private {
        // Consume before paying. The guard already blocks reentry; ordering it
        // this way means even a future refactor that drops the guard cannot
        // replay this settlement from inside a token callback.
        nonce = nonce_ + 1;
        emit Withdrawn(nonce_, epoch, payouts.length);

        for (uint256 i = 0; i < payouts.length; i++) {
            Payout calldata p = payouts[i];
            uint32 id = uint32(p.asset);
            if (uint256(id) != p.asset) revert UnknownAsset();
            Asset memory a = _asset(id);
            uint256 out = _fromWad(p.amount, a.decimals);
            if (out == 0) revert ZeroAmount();
            if (a.token == address(0)) {
                Address.sendValue(payable(p.to), out);
            } else {
                IERC20(a.token).safeTransfer(p.to, out);
            }
            emit PaidOut(p.to, id, out);
        }
    }

    // ------------------------------------------------------------- EIP-712

    function domainSeparator() public view returns (bytes32) {
        return keccak256(
            abi.encode(
                DOMAIN_TYPEHASH,
                keccak256("ZynVault"),
                keccak256("1"),
                block.chainid,
                address(this)
            )
        );
    }

    function withdrawalDigest(uint64 nonce_, uint64 epoch, Payout[] calldata payouts)
        public
        view
        returns (bytes32)
    {
        bytes32[] memory hashes = new bytes32[](payouts.length);
        for (uint256 i = 0; i < payouts.length; i++) {
            hashes[i] = keccak256(
                abi.encode(PAYOUT_TYPEHASH, payouts[i].to, payouts[i].asset, payouts[i].amount)
            );
        }
        bytes32 structHash = keccak256(
            abi.encode(
                WITHDRAWAL_TYPEHASH,
                uint256(nonce_),
                uint256(epoch),
                keccak256(abi.encodePacked(hashes))
            )
        );
        return keccak256(abi.encodePacked(hex"1901", domainSeparator(), structHash));
    }

    // -------------------------------------------------------------- scaling

    function _asset(uint32 asset) private view returns (Asset memory a) {
        a = assets[asset];
        if (!a.registered) revert UnknownAsset();
    }

    /// Token units -> WAD.
    function _toWad(uint256 amount, uint8 decimals) private pure returns (uint256) {
        if (decimals == 18) return amount;
        if (decimals < 18) return amount * (10 ** (18 - uint256(decimals)));
        uint256 d = 10 ** (uint256(decimals) - 18);
        if (amount % d != 0) revert InexactScaling();
        return amount / d;
    }

    /// WAD -> token units. Reverts rather than round: dropping dust silently is
    /// how a bridge's books stop balancing, and the VM's exit minimum is what
    /// keeps this from being reachable in practice.
    function _fromWad(uint256 wad, uint8 decimals) private pure returns (uint256) {
        if (decimals == 18) return wad;
        if (decimals > 18) return wad * (10 ** (uint256(decimals) - 18));
        uint256 d = 10 ** (18 - uint256(decimals));
        if (wad % d != 0) revert InexactScaling();
        return wad / d;
    }

    /// Native value arrives only through `depositNative`, where it is credited.
    /// A bare transfer would sit here uncounted, so it is refused.
    receive() external payable {
        revert NoDestination();
    }
}
