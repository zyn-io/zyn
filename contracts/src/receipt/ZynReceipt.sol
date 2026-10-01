// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ERC721} from "@openzeppelin/contracts/token/ERC721/ERC721.sol";
import {Base64} from "@openzeppelin/contracts/utils/Base64.sol";
import {Strings} from "@openzeppelin/contracts/utils/Strings.sol";

/// @title ZynReceipt — a record of a deposit into Zyn, as an NFT.
///
/// A receipt is a record, not a claim: holding it owns nothing on Zyn, and it
/// cannot be transferred (the holder may burn it). The receipt service mints
/// one per deposit after checking the deposit on this chain:
///
/// - **Transparent**: to the address that deposited, showing the asset and
///   the amount — what the chain already shows about that deposit.
/// - **Private**: to any address the depositor names, showing only a hash
///   that commits to the deposit's note with a salt only the depositor holds.
///   Nothing on it links the two addresses; the depositor can open it later.
///
/// Metadata and image are generated here, on-chain.
contract ZynReceipt is ERC721 {
    using Strings for uint256;

    struct Receipt {
        bool isPrivate;
        uint64 at;
        uint8 decimals;
        uint256 amount;
        string asset;
        string chainName;
        bytes32 hash;
    }

    address public minter;
    uint256 public minted;
    mapping(uint256 => Receipt) internal receipts;

    event Minter(address next);

    error NotMinter();
    error Locked();
    error AlreadyMinted();

    constructor(address minter_) ERC721("Zyn Deposit Receipt", "ZYNR") {
        minter = minter_;
    }

    modifier onlyMinter() {
        if (msg.sender != minter) revert NotMinter();
        _;
    }

    function setMinter(address next) external onlyMinter {
        minter = next;
        emit Minter(next);
    }

    /// The id of a transparent receipt: one per deposit transaction.
    function transparentId(bytes32 depositTx) public pure returns (uint256) {
        return uint256(keccak256(abi.encodePacked("zyn-receipt.transparent.v1", depositTx)));
    }

    /// The id of a private receipt: one per hash.
    function privateId(bytes32 hash) public pure returns (uint256) {
        return uint256(keccak256(abi.encodePacked("zyn-receipt.private.v1", hash)));
    }

    function mintTransparent(address to, bytes32 depositTx, string calldata asset, string calldata chainName, uint256 amount, uint8 decimals) external onlyMinter returns (uint256 id) {
        id = transparentId(depositTx);
        if (_ownerOf(id) != address(0)) revert AlreadyMinted();
        receipts[id] = Receipt(false, uint64(block.timestamp), decimals, amount, asset, chainName, bytes32(0));
        minted += 1;
        _mint(to, id);
    }

    function mintPrivate(address to, bytes32 hash, string calldata chainName) external onlyMinter returns (uint256 id) {
        id = privateId(hash);
        if (_ownerOf(id) != address(0)) revert AlreadyMinted();
        receipts[id] = Receipt(true, uint64(block.timestamp), 0, 0, "", chainName, hash);
        minted += 1;
        _mint(to, id);
    }

    /// The holder may throw a receipt away; nobody may move one.
    function burn(uint256 id) external {
        if (ownerOf(id) != msg.sender) revert Locked();
        _burn(id);
        delete receipts[id];
    }

    function _update(address to, uint256 id, address auth) internal override returns (address from) {
        from = _ownerOf(id);
        if (from != address(0) && to != address(0)) revert Locked();
        return super._update(to, id, auth);
    }

    function receipt(uint256 id) external view returns (Receipt memory) {
        _requireOwned(id);
        return receipts[id];
    }

    /// `amount` with `decimals` places, trailing zeros dropped: "0.5".
    function amountText(uint256 amount, uint8 decimals) public pure returns (string memory) {
        uint256 unit = 10 ** decimals;
        uint256 whole = amount / unit;
        uint256 frac = amount % unit;
        if (frac == 0) return whole.toString();
        bytes memory f = bytes(frac.toString());
        bytes memory padded = new bytes(decimals);
        for (uint256 i = 0; i < decimals; i++) padded[i] = "0";
        for (uint256 i = 0; i < f.length; i++) padded[decimals - f.length + i] = f[i];
        uint256 end = decimals;
        while (end > 0 && padded[end - 1] == "0") end--;
        bytes memory out = new bytes(end);
        for (uint256 i = 0; i < end; i++) out[i] = padded[i];
        return string.concat(whole.toString(), ".", string(out));
    }

    function _image(Receipt memory r) internal pure returns (string memory) {
        string memory line = r.isPrivate
            ? string.concat(_short(r.hash), "</text>")
            : string.concat(amountText(r.amount, r.decimals), " ", r.asset, "</text>");
        return string.concat(
            "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 600 600'>",
            "<defs><linearGradient id='g' x1='0' y1='0' x2='1' y2='1'><stop offset='0' stop-color='#0f2a22'/><stop offset='1' stop-color='#06110e'/></linearGradient>",
            "<linearGradient id='j' x1='0' y1='0' x2='1' y2='0'><stop offset='0' stop-color='#0b7a61'/><stop offset='1' stop-color='#58e8bb'/></linearGradient></defs>",
            "<rect width='600' height='600' fill='url(#g)'/>",
            "<rect x='40' y='40' width='520' height='520' fill='none' stroke='rgba(242,241,235,0.14)'/>",
            "<text x='72' y='112' fill='#f2f1eb' font-family='Arial Black,Arial' font-size='44' font-weight='900'>ZYN</text>",
            "<text x='72' y='150' fill='#8a9691' font-family='monospace' font-size='18' letter-spacing='3'>",
            r.isPrivate ? "PRIVATE DEPOSIT" : "DEPOSIT RECEIPT",
            "</text><rect x='72' y='300' width='456' height='4' fill='url(#j)'/>",
            "<text x='72' y='380' fill='#f2f1eb' font-family='monospace' font-size='",
            r.isPrivate ? "22" : "40",
            "'>",
            line,
            "<text x='72' y='500' fill='#8a9691' font-family='monospace' font-size='18'>",
            r.chainName,
            "</text></svg>"
        );
    }

    function _short(bytes32 h) internal pure returns (string memory) {
        string memory s = Strings.toHexString(uint256(h), 32);
        bytes memory b = bytes(s);
        bytes memory out = new bytes(23);
        for (uint256 i = 0; i < 12; i++) out[i] = b[i];
        out[12] = 0xE2; out[13] = 0x80; out[14] = 0xA6; // …
        for (uint256 i = 0; i < 8; i++) out[15 + i] = b[b.length - 8 + i];
        return string(out);
    }

    function tokenURI(uint256 id) public view override returns (string memory) {
        _requireOwned(id);
        Receipt memory r = receipts[id];
        string memory attrs = r.isPrivate
            ? string.concat('{"trait_type":"Channel","value":"Private"},{"trait_type":"Hash","value":"', Strings.toHexString(uint256(r.hash), 32), '"}')
            : string.concat(
                '{"trait_type":"Channel","value":"Transparent"},{"trait_type":"Asset","value":"', r.asset,
                '"},{"trait_type":"Amount","value":"', amountText(r.amount, r.decimals), '"}'
            );
        string memory json = string.concat(
            '{"name":"', r.isPrivate ? "Zyn private deposit" : string.concat("Zyn deposit: ", amountText(r.amount, r.decimals), " ", r.asset),
            '","description":"', r.isPrivate
                ? "A record of a private deposit into Zyn. It shows only a hash; its holder can open it to prove which deposit it records. It owns nothing on Zyn."
                : "A record of a deposit into Zyn from this address. It owns nothing on Zyn.",
            '","image":"data:image/svg+xml;base64,', Base64.encode(bytes(_image(r))),
            '","attributes":[', attrs, ',{"trait_type":"Chain","value":"', r.chainName,
            '"},{"display_type":"date","trait_type":"Recorded","value":', uint256(r.at).toString(), '}]}'
        );
        return string.concat("data:application/json;base64,", Base64.encode(bytes(json)));
    }
}
