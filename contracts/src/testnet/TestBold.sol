// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";

/// Testnet stand-in for Liquity V2's BOLD (DECISIONS §68), so BOLD.zy can be
/// bridged, lent and liquidated on Zyn's testnet without opening a Liquity
/// trove. Anyone may mint; it is worth nothing. Mainnet bridges only the
/// canonical BOLD, 0x6440f144b7e50D6a8439336510312d2F54beB01D (§68.1).
contract TestBold is ERC20 {
    /// At most this much per call, so a faucet script cannot overflow a pool.
    uint256 public constant MINT_LIMIT = 1_000_000e18;

    error TooMuch();

    constructor() ERC20("Test BOLD (Zyn testnet)", "BOLD") {}

    function mint(address to, uint256 amount) external {
        if (amount > MINT_LIMIT) revert TooMuch();
        _mint(to, amount);
    }
}
