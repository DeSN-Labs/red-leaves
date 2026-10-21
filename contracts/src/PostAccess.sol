// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

/// @notice Testnet native-ETH purchases. Every stored byte is PUBLIC.
/// Encrypted keys belong to the author's NAS, never to an unpaid buyer.
contract PostAccess {
    struct Listing {
        address payable author;
        uint128 price;
        bytes encryptedKey;
    }
    mapping(address => mapping(bytes32 => Listing)) private listings;
    mapping(address => mapping(bytes32 => mapping(address => bool))) public purchased;
    error InvalidListing();
    error AlreadyListed();
    error InvalidPurchase();
    error PaymentFailed();
    event Listed(bytes32 indexed postId, address indexed author, uint128 price);
    event Purchased(bytes32 indexed postId, address indexed buyer, uint128 price);

    function registerPost(bytes32 postId, uint128 price, bytes calldata encryptedKey) external {
        if (postId == bytes32(0) || price == 0 || encryptedKey.length == 0 || encryptedKey.length > 2048) {
            revert InvalidListing();
        }
        if (listings[msg.sender][postId].author != address(0)) revert AlreadyListed();
        listings[msg.sender][postId] = Listing(payable(msg.sender), price, encryptedKey);
        emit Listed(postId, msg.sender, price);
    }

    function getPost(address owner, bytes32 postId)
        external
        view
        returns (address author, uint128 price, bytes memory encryptedKey)
    {
        Listing storage listing = listings[owner][postId];
        return (listing.author, listing.price, listing.encryptedKey);
    }

    function buyPost(address owner, bytes32 postId) external payable {
        Listing storage listing = listings[owner][postId];
        if (
            listing.author == address(0) || msg.sender == listing.author || purchased[owner][postId][msg.sender]
                || msg.value != listing.price
        ) revert InvalidPurchase();
        purchased[owner][postId][msg.sender] = true;
        (bool sent,) = listing.author.call{value: msg.value}("");
        if (!sent) revert PaymentFailed();
        emit Purchased(postId, msg.sender, listing.price);
    }
}
