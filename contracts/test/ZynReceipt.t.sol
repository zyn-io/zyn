// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {ZynReceipt} from "../src/receipt/ZynReceipt.sol";

contract ZynReceiptTest is Test {
    ZynReceipt r;
    address minter = address(0xA11CE);
    address holder = address(0xB0B);

    function setUp() public {
        r = new ZynReceipt(minter);
    }

    function test_only_the_minter_mints_and_once_per_deposit() public {
        vm.expectRevert(ZynReceipt.NotMinter.selector);
        r.mintTransparent(holder, bytes32(uint256(1)), "ETH", "Sepolia", 5e17, 18);
        vm.prank(minter);
        uint256 id = r.mintTransparent(holder, bytes32(uint256(1)), "ETH", "Sepolia", 5e17, 18);
        assertEq(r.ownerOf(id), holder);
        assertEq(id, r.transparentId(bytes32(uint256(1))));
        vm.prank(minter);
        vm.expectRevert(ZynReceipt.AlreadyMinted.selector);
        r.mintTransparent(holder, bytes32(uint256(1)), "ETH", "Sepolia", 5e17, 18);
        assertEq(r.minted(), 1);
    }

    function test_a_receipt_cannot_move_but_its_holder_can_burn_it() public {
        vm.prank(minter);
        uint256 id = r.mintPrivate(holder, keccak256("note"), "Sepolia");
        vm.prank(holder);
        vm.expectRevert(ZynReceipt.Locked.selector);
        r.transferFrom(holder, address(0xCAFE), id);
        vm.prank(address(0xCAFE));
        vm.expectRevert(ZynReceipt.Locked.selector);
        r.burn(id);
        vm.prank(holder);
        r.burn(id);
        vm.expectRevert();
        r.ownerOf(id);
    }

    function test_amounts_read_as_people_write_them() public view {
        assertEq(r.amountText(5e17, 18), "0.5");
        assertEq(r.amountText(1e18, 18), "1");
        assertEq(r.amountText(1234500000, 9), "1.2345");
        assertEq(r.amountText(1, 8), "0.00000001");
    }

    function test_metadata_shows_the_amount_or_only_the_hash() public {
        vm.startPrank(minter);
        uint256 t = r.mintTransparent(holder, bytes32(uint256(7)), "ETH", "Sepolia", 25e16, 18);
        uint256 p = r.mintPrivate(holder, bytes32(uint256(0xabc)), "Solana devnet");
        vm.stopPrank();
        string memory tu = r.tokenURI(t);
        string memory pu = r.tokenURI(p);
        assertTrue(bytes(tu).length > 200 && bytes(pu).length > 200);
        ZynReceipt.Receipt memory rt = r.receipt(t);
        assertEq(rt.amount, 25e16);
        assertFalse(rt.isPrivate);
        ZynReceipt.Receipt memory rp = r.receipt(p);
        assertTrue(rp.isPrivate);
        assertEq(rp.amount, 0, "a private receipt carries no amount");
        emit log(tu);
    }
}
