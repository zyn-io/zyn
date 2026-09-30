// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {ZynPool} from "../src/pool/ZynPool.sol";
import {PoseidonT3} from "../src/pool/PoseidonT3.sol";

/// The EVM pool against proofs made by the Rust circuit (zyn-pool-sol writes
/// test/fixtures/pool.json): the same notes, root and proofs the Solana
/// program takes.
contract ZynPoolTest is Test {
    ZynPool pool;
    address authority = address(0xA11CE);
    string fx;
    uint256 constant SIZE = 0.1 ether;

    function setUp() public {
        fx = vm.readFile("test/fixtures/pool.json");
        pool = new ZynPool(SIZE, authority);
        vm.deal(address(this), 10 ether);
    }

    function _u(string memory key) internal view returns (uint256) {
        return vm.parseJsonUint(fx, key);
    }

    function _deposits() internal {
        uint256[] memory cs = vm.parseJsonUintArray(fx, ".commitments");
        for (uint256 i = 0; i < cs.length; i++) pool.deposit{value: SIZE}(cs[i]);
    }

    function test_poseidon_is_circoms() public pure {
        assertEq(PoseidonT3.hash(1, 2), 0x115cc0f5e7d690413df64c6b9662e9cf2a3617f2743245519e19607a4417189a);
    }

    function test_the_tree_matches_the_rust_tree() public {
        uint256[] memory z = vm.parseJsonUintArray(fx, ".zeros");
        for (uint256 i = 0; i <= 20; i++) assertEq(pool.zeros(i), z[i], "empty subtrees");
        _deposits();
        assertTrue(pool.isKnownRoot(_u(".root")), "the root after three deposits is the Rust root");
    }

    function test_deposit_takes_exactly_one_size() public {
        vm.expectRevert(ZynPool.WrongAmount.selector);
        pool.deposit{value: SIZE - 1}(1);
    }

    function test_spend_into_zyn_once_and_the_coin_stays() public {
        _deposits();
        bytes memory proof = vm.parseJsonBytes(fx, ".toZynProof");
        bytes32 code = vm.parseJsonBytes32(fx, ".code");
        uint256 root = _u(".root");
        uint256 nh = _u(".nullifierHash");
        uint256 g = gasleft();
        pool.spendToZyn(proof, root, nh, code);
        emit log_named_uint("spendToZyn gas", g - gasleft());
        assertEq(address(pool).balance, 3 * SIZE, "the coin stays; it backs the bridged asset");
        assertEq(pool.live(), 2);
        vm.expectRevert(ZynPool.AlreadySpent.selector);
        pool.spendToZyn(proof, root, nh, code);
    }

    function test_a_proof_cannot_be_pointed_elsewhere() public {
        _deposits();
        bytes memory proof = vm.parseJsonBytes(fx, ".toZynProof");
        vm.expectRevert(ZynPool.BadProof.selector);
        pool.spendToZyn(proof, _u(".root"), _u(".nullifierHash"), bytes32(uint256(0x43)));
        vm.expectRevert(ZynPool.UnknownRoot.selector);
        pool.spendToZyn(proof, 12345, _u(".nullifierHash"), vm.parseJsonBytes32(fx, ".code"));
    }

    function test_spend_to_a_fresh_address_through_a_relayer() public {
        _deposits();
        bytes memory proof = vm.parseJsonBytes(fx, ".toAddressProof");
        address payable recipient = payable(vm.parseJsonAddress(fx, ".recipient"));
        address payable relayer = payable(vm.parseJsonAddress(fx, ".relayer"));
        uint256 fee = _u(".fee");
        // Another recipient, or a higher fee: refused.
        vm.expectRevert(ZynPool.BadProof.selector);
        pool.spendToAddress(proof, _u(".root"), _u(".nullifierHash"), payable(address(0xBAD)), relayer, fee);
        vm.expectRevert(ZynPool.BadProof.selector);
        pool.spendToAddress(proof, _u(".root"), _u(".nullifierHash"), recipient, relayer, fee * 2);
        uint256 g = gasleft();
        pool.spendToAddress(proof, _u(".root"), _u(".nullifierHash"), recipient, relayer, fee);
        emit log_named_uint("spendToAddress gas", g - gasleft());
        assertEq(recipient.balance, SIZE - fee, "the recipient gets the note less the fee");
        assertEq(relayer.balance, fee);
    }

    function test_exits_come_from_the_authority_and_never_outrun_the_pool() public {
        _deposits(); // live 3, balance 3 sizes
        vm.expectRevert(ZynPool.NotAuthority.selector);
        pool.insertExit(7);
        vm.prank(authority);
        vm.expectRevert(ZynPool.Underfunded.selector);
        pool.insertExit(7);
        // A spend into Zyn frees one size to back an exit.
        pool.spendToZyn(vm.parseJsonBytes(fx, ".toZynProof"), _u(".root"), _u(".nullifierHash"), vm.parseJsonBytes32(fx, ".code"));
        vm.prank(authority);
        pool.insertExit(7);
        assertEq(pool.live(), 3);
        // A top-up from the vault backs another.
        (bool ok,) = address(pool).call{value: SIZE}("");
        assertTrue(ok);
        vm.prank(authority);
        pool.insertExit(8);
    }

    function test_the_authority_hands_itself_on() public {
        vm.expectRevert(ZynPool.NotAuthority.selector);
        pool.setAuthority(address(this));
        vm.prank(authority);
        pool.setAuthority(address(0xB0B));
        assertEq(pool.authority(), address(0xB0B));
    }

    function test_deposit_gas() public {
        uint256 g = gasleft();
        pool.deposit{value: SIZE}(1);
        emit log_named_uint("deposit gas", g - gasleft());
    }

    receive() external payable {}
}
