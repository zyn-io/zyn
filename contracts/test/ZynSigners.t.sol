// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {ZynVault} from "../src/ZynVault.sol";
import {ZynSigners} from "../src/ZynSigners.sol";

/// ETH.zy's vault on Sepolia (DECISIONS §146): native ETH in against a Nap
/// commitment, out only with two of three owners' signatures.
contract ZynSignersTest is Test {
    uint256[3] pk = [uint256(0xA1), uint256(0xB2), uint256(0xC3)];
    address[3] who;
    ZynSigners signers;
    ZynVault vault;
    uint32 constant ETH = 1;

    function setUp() public {
        vm.chainId(11155111);
        address[] memory owners = new address[](3);
        for (uint256 i = 0; i < 3; i++) {
            who[i] = vm.addr(pk[i]);
            owners[i] = who[i];
        }
        signers = new ZynSigners(owners, 2);
        vault = new ZynVault(address(signers), address(this));
        vault.registerAsset(ETH, address(0));
    }

    function sig(uint256 k, bytes32 digest) internal pure returns (bytes memory) {
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(k, digest);
        return abi.encodePacked(r, s, v);
    }

    /// Two signatures, ascending by signer address.
    function two(uint256 a, uint256 b, bytes32 digest) internal view returns (bytes memory) {
        (uint256 lo, uint256 hi) = vm.addr(a) < vm.addr(b) ? (a, b) : (b, a);
        return bytes.concat(sig(lo, digest), sig(hi, digest));
    }

    function payouts(address to, uint256 wad) internal pure returns (ZynVault.Payout[] memory p) {
        p = new ZynVault.Payout[](1);
        p[0] = ZynVault.Payout({to: to, asset: ETH, amount: wad});
    }

    function test_a_commitment_deposits_and_two_of_three_withdraw() public {
        bytes32 commitment = keccak256("nap commitment");
        vm.expectEmit(true, true, false, true, address(vault));
        emit ZynVault.Deposited(commitment, ETH, 1 ether, 0);
        vault.depositNative{value: 1 ether}(commitment, ETH);

        address to = address(0xBEEF);
        ZynVault.Payout[] memory p = payouts(to, 0.4 ether);
        bytes32 digest = vault.withdrawalDigest(0, 7, p);
        vault.withdraw(0, 7, p, two(pk[0], pk[2], digest));
        assertEq(to.balance, 0.4 ether);
        assertEq(vault.nonce(), 1);
    }

    function test_one_key_a_repeated_key_a_stranger_or_the_wrong_order_is_not_enough() public {
        vault.depositNative{value: 1 ether}(bytes32(uint256(1)), ETH);
        ZynVault.Payout[] memory p = payouts(address(0xBEEF), 0.1 ether);
        bytes32 digest = vault.withdrawalDigest(0, 0, p);

        vm.expectRevert(ZynVault.BadSignature.selector);
        vault.withdraw(0, 0, p, sig(pk[0], digest));

        vm.expectRevert(ZynVault.BadSignature.selector);
        vault.withdraw(0, 0, p, bytes.concat(sig(pk[1], digest), sig(pk[1], digest)));

        vm.expectRevert(ZynVault.BadSignature.selector);
        vault.withdraw(0, 0, p, two(pk[0], 0xDEAD, digest));

        (uint256 lo, uint256 hi) = who[0] < who[1] ? (pk[0], pk[1]) : (pk[1], pk[0]);
        vm.expectRevert(ZynVault.BadSignature.selector);
        vault.withdraw(0, 0, p, bytes.concat(sig(hi, digest), sig(lo, digest)));

        // And a valid pair cannot be replayed.
        vault.withdraw(0, 0, p, two(pk[1], pk[2], digest));
        vm.expectRevert(ZynVault.BadNonce.selector);
        vault.withdraw(0, 0, p, two(pk[1], pk[2], digest));
    }

    function test_the_set_is_fixed_and_sane() public {
        address[] memory none = new address[](0);
        vm.expectRevert(bytes("threshold"));
        new ZynSigners(none, 1);
        address[] memory dup = new address[](2);
        dup[0] = who[0];
        dup[1] = who[0];
        vm.expectRevert(bytes("owner"));
        new ZynSigners(dup, 1);
        assertEq(signers.threshold(), 2);
        assertEq(signers.owners().length, 3);
    }
}
