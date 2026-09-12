// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {Vm} from "forge-std/Vm.sol";
import {ZynVault} from "../src/ZynVault.sol";
import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";

contract Token is ERC20 {
    uint8 private immutable d;

    constructor(string memory n, uint8 decimals_) ERC20(n, n) {
        d = decimals_;
    }

    function decimals() public view override returns (uint8) {
        return d;
    }

    function mint(address to, uint256 v) external {
        _mint(to, v);
    }
}

/// Takes a 1% cut on transfer — the case that makes crediting the *requested*
/// amount issue units the vault does not hold.
contract FeeToken is ERC20 {
    constructor() ERC20("Fee", "FEE") {}

    function mint(address to, uint256 v) external {
        _mint(to, v);
    }

    function _update(address from, address to, uint256 v) internal override {
        if (from != address(0) && to != address(0)) {
            uint256 fee = v / 100;
            super._update(from, address(0xdead), fee);
            v -= fee;
        }
        super._update(from, to, v);
    }
}

contract ZynVaultTest is Test {
    /// The address the Rust vector was generated for.
    address constant VAULT_ADDR = 0xABaBaBaBABabABabAbAbABAbABabababaBaBABaB;
    uint256 constant CHAIN = 8453; // Base
    uint32 constant ASSET = 3;

    ZynVault vault;
    Token token;
    uint256 signerPk = 0xA11CE;
    address signerAddr;
    address admin = address(0xAD);

    function setUp() public {
        vm.chainId(CHAIN);
        signerAddr = vm.addr(signerPk);
        deployCodeTo("ZynVault.sol:ZynVault", abi.encode(signerAddr, admin), VAULT_ADDR);
        vault = ZynVault(payable(VAULT_ADDR));
        token = new Token("Wrapped", 18);
        vm.prank(admin);
        vault.registerAsset(ASSET, address(token));
    }

    function _one(address to, uint256 amount)
        internal
        pure
        returns (ZynVault.Payout[] memory p)
    {
        p = new ZynVault.Payout[](1);
        p[0] = ZynVault.Payout({to: to, asset: ASSET, amount: amount});
    }

    // ------------------------------------------------- the cross-language check

    /// The only test that proves the Rust signer and this verifier agree.
    /// Everything else could pass with the two implementations quietly
    /// producing different digests, which on a live chain means every
    /// withdrawal reverts and the vault is bricked.
    ///
    /// Vector from `zyn_bridge::evm::tests::the_type_strings_are_what_the_contract_expects`.
    function test_digest_matches_the_rust_vector() public view {
        ZynVault.Payout[] memory p = _one(address(0x0101010101010101010101010101010101010101), 5e18);
        assertEq(
            vault.withdrawalDigest(1, 10, p),
            0x3914e0bb5d68693e0bae52a1c8cab270d0069f43843109efdba71359417db801,
            "Rust and Solidity disagree on the withdrawal digest"
        );
    }

    function test_domain_separator_matches_the_rust_vector() public view {
        assertEq(
            vault.domainSeparator(),
            0xa34f076f7012dd1cbad50cf12e83e35e6342e6d1dcdb5636205bbc9ad8f54214
        );
    }

    function test_typehashes_match() public view {
        assertEq(
            vault.WITHDRAWAL_TYPEHASH(),
            keccak256(
                "Withdrawal(uint256 nonce,uint256 epoch,Payout[] payouts)Payout(address to,uint256 asset,uint256 amount)"
            )
        );
        assertEq(
            vault.PAYOUT_TYPEHASH(),
            keccak256("Payout(address to,uint256 asset,uint256 amount)")
        );
    }

    /// The Rust watcher filters on a hard-coded topic. If the event signature
    /// here ever changes, that filter matches nothing — no deposit is credited,
    /// forever, and no error is raised anywhere. Asserted against a log this
    /// contract actually emitted rather than against a string, so a change to
    /// the parameter types is caught too.
    ///
    /// Pinned in `zyn-custody/src/evm.rs` as `DEPOSITED_TOPIC`.
    function test_the_deposit_topic_matches_the_watcher() public {
        token.mint(address(this), 1e18);
        token.approve(address(vault), 1e18);

        vm.recordLogs();
        vault.deposit(keccak256("acct"), ASSET, 1e18);
        Vm.Log[] memory logs = vm.getRecordedLogs();

        bytes32 topic;
        for (uint256 i = 0; i < logs.length; i++) {
            if (logs[i].emitter == VAULT_ADDR) topic = logs[i].topics[0];
        }
        assertEq(
            topic,
            0xb1da2eb4f778a6fb950403ef78441a12e911b50b524dcc8abbdf4d85e36c9849,
            "the Rust watcher is filtering on a topic this contract no longer emits"
        );
    }

    // ----------------------------------------------------------------- deposits

    function test_a_deposit_names_its_destination() public {
        bytes32 dest = keccak256("some zyn account");
        token.mint(address(this), 10e18);
        token.approve(address(vault), 10e18);

        vm.expectEmit(true, true, false, true, address(vault));
        emit ZynVault.Deposited(dest, ASSET, 10e18, 0);
        vault.deposit(dest, ASSET, 10e18);

        assertEq(vault.depositIndex(ASSET), 1, "the index must advance");
    }

    /// The reason this is a contract at all: the destination is unrelated to
    /// the sender, so depositing does not link an EVM identity to a Zyn one.
    function test_the_destination_need_not_be_the_sender() public {
        token.mint(address(this), 1e18);
        token.approve(address(vault), 1e18);
        vault.deposit(keccak256("a stranger's account"), ASSET, 1e18);
        assertEq(vault.depositIndex(ASSET), 1);
    }

    function test_a_deposit_credits_what_arrived_not_what_was_asked() public {
        FeeToken fee = new FeeToken();
        vm.prank(admin);
        vault.registerAsset(9, address(fee));
        fee.mint(address(this), 100e18);
        fee.approve(address(vault), 100e18);

        vm.expectEmit(true, true, false, true, address(vault));
        emit ZynVault.Deposited(keccak256("acct"), 9, 99e18, 0); // 1% taken
        vault.deposit(keccak256("acct"), 9, 100e18);
    }

    function test_an_unregistered_asset_is_refused() public {
        vm.expectRevert(ZynVault.UnknownAsset.selector);
        vault.deposit(keccak256("acct"), 77, 1e18);
    }

    function test_a_bare_transfer_is_refused() public {
        vm.deal(address(this), 1 ether);
        (bool ok,) = VAULT_ADDR.call{value: 1 ether}("");
        assertFalse(ok, "value arrived without a destination and was kept");
    }

    // -------------------------------------------------------------- withdrawals

    function _sign(uint64 nonce_, uint64 epoch, ZynVault.Payout[] memory p)
        internal
        view
        returns (bytes memory)
    {
        (uint8 v, bytes32 r, bytes32 s) =
            vm.sign(signerPk, vault.withdrawalDigest(nonce_, epoch, p));
        return abi.encodePacked(r, s, v);
    }

    function test_a_signed_withdrawal_pays_out() public {
        token.mint(address(vault), 10e18);
        ZynVault.Payout[] memory p = _one(address(0xBEEF), 4e18);
        vault.withdraw(0, 7, p, _sign(0, 7, p));
        assertEq(token.balanceOf(address(0xBEEF)), 4e18);
        assertEq(vault.nonce(), 1);
    }

    function test_an_unsigned_withdrawal_is_refused() public {
        token.mint(address(vault), 10e18);
        ZynVault.Payout[] memory p = _one(address(0xBEEF), 4e18);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(0xB0B, vault.withdrawalDigest(0, 7, p));
        vm.expectRevert(ZynVault.BadSignature.selector);
        vault.withdraw(0, 7, p, abi.encodePacked(r, s, v));
    }

    /// A signature covers one payout list. Editing the recipient after signing
    /// is the attack a bare "operator broadcasts a transaction" model allows.
    function test_the_payout_list_cannot_be_edited_after_signing() public {
        token.mint(address(vault), 10e18);
        ZynVault.Payout[] memory p = _one(address(0xBEEF), 4e18);
        bytes memory sig = _sign(0, 7, p);
        p[0].to = address(0xBAD);
        vm.expectRevert(ZynVault.BadSignature.selector);
        vault.withdraw(0, 7, p, sig);
    }

    function test_a_withdrawal_cannot_be_replayed() public {
        token.mint(address(vault), 10e18);
        ZynVault.Payout[] memory p = _one(address(0xBEEF), 4e18);
        bytes memory sig = _sign(0, 7, p);
        vault.withdraw(0, 7, p, sig);
        vm.expectRevert(ZynVault.BadNonce.selector);
        vault.withdraw(0, 7, p, sig);
    }

    /// The CREATE2 consequence. The same vault address exists on Arbitrum, so
    /// a Base signature must not settle there.
    function test_a_signature_does_not_cross_chains() public {
        token.mint(address(vault), 10e18);
        ZynVault.Payout[] memory p = _one(address(0xBEEF), 4e18);
        bytes memory sig = _sign(0, 7, p);

        vm.chainId(42161); // Arbitrum, same address
        vm.expectRevert(ZynVault.BadSignature.selector);
        vault.withdraw(0, 7, p, sig);
    }

    /// Anyone may push a signed withdrawal through, so leaving does not depend
    /// on a particular operator choosing to broadcast.
    function test_anyone_may_submit_a_signed_withdrawal() public {
        token.mint(address(vault), 10e18);
        ZynVault.Payout[] memory p = _one(address(0xBEEF), 4e18);
        bytes memory sig = _sign(0, 7, p);
        vm.prank(address(0xF00D));
        vault.withdraw(0, 7, p, sig);
        assertEq(token.balanceOf(address(0xBEEF)), 4e18);
    }

    // ------------------------------------------------------------------ scaling

    function test_a_six_decimal_token_scales_exactly() public {
        Token usdc = new Token("USDC", 6);
        vm.prank(admin);
        vault.registerAsset(6, address(usdc));
        usdc.mint(address(this), 1_000e6);
        usdc.approve(address(vault), 1_000e6);

        vm.expectEmit(true, true, false, true, address(vault));
        emit ZynVault.Deposited(keccak256("acct"), 6, 1_000e18, 0);
        vault.deposit(keccak256("acct"), 6, 1_000e6);

        usdc.mint(address(vault), 500e6);
        ZynVault.Payout[] memory p = new ZynVault.Payout[](1);
        p[0] = ZynVault.Payout({to: address(0xBEEF), asset: 6, amount: 500e18});
        vault.withdraw(0, 7, p, _sign(0, 7, p));
        assertEq(usdc.balanceOf(address(0xBEEF)), 500e6);
    }

    /// Dust that a 6-decimal token cannot express must stop the withdrawal
    /// rather than be rounded away — silently dropped dust is how the books
    /// stop balancing.
    function test_an_inexpressible_amount_reverts_rather_than_rounds() public {
        Token usdc = new Token("USDC", 6);
        vm.prank(admin);
        vault.registerAsset(6, address(usdc));
        usdc.mint(address(vault), 1_000e6);

        ZynVault.Payout[] memory p = new ZynVault.Payout[](1);
        p[0] = ZynVault.Payout({to: address(0xBEEF), asset: 6, amount: 1e18 + 1});
        // Signed first: `expectRevert` binds to the next call, and building the
        // signature is itself a call into the vault.
        bytes memory sig = _sign(0, 7, p);
        vm.expectRevert(ZynVault.InexactScaling.selector);
        vault.withdraw(0, 7, p, sig);
    }

    // -------------------------------------------------------------------- admin

    /// Add-only. Re-pointing a live asset id at another token would let this
    /// role rewrite what deposited value is.
    function test_an_asset_cannot_be_repointed() public {
        Token other = new Token("Other", 18);
        vm.prank(admin);
        vm.expectRevert(ZynVault.AlreadyRegistered.selector);
        vault.registerAsset(ASSET, address(other));
    }

    function test_only_the_admin_registers_assets() public {
        vm.expectRevert(ZynVault.NotAdmin.selector);
        vault.registerAsset(42, address(token));
    }

    /// Deposits pause; withdrawals do not. No role here can trap funds.
    function test_pausing_deposits_does_not_pause_exits() public {
        vm.prank(admin);
        vault.pauseDeposits(true);

        token.mint(address(this), 1e18);
        token.approve(address(vault), 1e18);
        vm.expectRevert(ZynVault.DepositsArePaused.selector);
        vault.deposit(keccak256("acct"), ASSET, 1e18);

        token.mint(address(vault), 10e18);
        ZynVault.Payout[] memory p = _one(address(0xBEEF), 4e18);
        vault.withdraw(0, 7, p, _sign(0, 7, p));
        assertEq(token.balanceOf(address(0xBEEF)), 4e18, "a pause trapped an exit");
    }
}
