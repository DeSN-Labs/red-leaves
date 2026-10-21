// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;
import {PostAccess} from "../src/PostAccess.sol";

interface PayVm {
    function prank(address) external;
    function deal(address, uint256) external;
    function expectRevert(bytes4) external;
}

contract PostAccessTest {
    PayVm private constant vm = PayVm(address(uint160(uint256(keccak256("hevm cheat code")))));
    PostAccess private market = new PostAccess();
    bytes32 private constant ID = keccak256("post");
    address private constant AUTHOR = address(0x1234);
    address private constant BUYER = address(0x5678);

    function list() internal {
        vm.prank(AUTHOR);
        market.registerPost(ID, 100, hex"abcd");
    }

    function testPurchasePaysAuthorAndGrantsOnlyBuyer() public {
        list();
        vm.deal(BUYER, 100);
        vm.prank(BUYER);
        market.buyPost{value: 100}(AUTHOR, ID);
        require(
            AUTHOR.balance == 100 && market.purchased(AUTHOR, ID, BUYER) && !market.purchased(AUTHOR, ID, address(1))
        );
    }

    function testWrongPriceAndReplayFail() public {
        list();
        vm.deal(BUYER, 300);
        vm.expectRevert(PostAccess.InvalidPurchase.selector);
        vm.prank(BUYER);
        market.buyPost{value: 99}(AUTHOR, ID);
        vm.prank(BUYER);
        market.buyPost{value: 100}(AUTHOR, ID);
        vm.expectRevert(PostAccess.InvalidPurchase.selector);
        vm.prank(BUYER);
        market.buyPost{value: 100}(AUTHOR, ID);
    }

    function testCannotReplacePriceOrEncryptedKey() public {
        list();
        vm.expectRevert(PostAccess.AlreadyListed.selector);
        vm.prank(AUTHOR);
        market.registerPost(ID, 200, hex"cdef");
        (address author, uint128 price, bytes memory ciphertext) = market.getPost(AUTHOR, ID);
        require(author == AUTHOR && price == 100 && keccak256(ciphertext) == keccak256(hex"abcd"));
    }

    function testInvalidListingFails() public {
        vm.expectRevert(PostAccess.InvalidListing.selector);
        market.registerPost(ID, 0, hex"abcd");
    }

    function testAnotherWalletCannotReserveAuthorsNamespace() public {
        vm.prank(BUYER);
        market.registerPost(ID, 200, hex"cdef");
        list();
        (address author, uint128 price,) = market.getPost(AUTHOR, ID);
        require(author == AUTHOR && price == 100);
    }

    function testPayoutRejectionRollsBackPurchase() public {
        RejectingSeller seller = new RejectingSeller(market, ID);
        vm.deal(BUYER, 100);
        vm.expectRevert(PostAccess.PaymentFailed.selector);
        vm.prank(BUYER);
        market.buyPost{value: 100}(address(seller), ID);
        require(!market.purchased(address(seller), ID, BUYER) && BUYER.balance == 100);
    }
}

contract RejectingSeller {
    constructor(PostAccess market, bytes32 id) {
        market.registerPost(id, 100, hex"abcd");
    }

    receive() external payable {
        revert("Payout rejected");
    }
}
