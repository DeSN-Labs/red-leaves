// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;
import {UserRegistry} from "../src/UserRegistry.sol";
import {DeployUserRegistry} from "../script/DeployUserRegistry.s.sol";

interface ITestVm {
    function prank(address sender) external;
    function expectRevert(bytes4 selector) external;
    function expectRevert(bytes calldata reason) external;
    function chainId(uint256 chainId) external;
    function expectEmit(bool topic1, bool topic2, bool topic3, bool data) external;
}

contract UserRegistryTest {
    ITestVm private constant VM = ITestVm(address(uint160(uint256(keccak256("hevm cheat code")))));
    UserRegistry private registry;
    address private constant ALICE = address(0xA11CE);
    address private constant BOB = address(0xB0B);

    function setUp() public {
        registry = new UserRegistry();
    }

    function registerAlice() private {
        VM.prank(ALICE);
        registry.register("peer-alice", hex"010203", "Alice");
    }

    function assertAliceUnchanged() private view {
        UserRegistry.User memory user = registry.getUser(ALICE);
        require(keccak256(bytes(user.nasPeerId)) == keccak256("peer-alice"), "PeerId mismatch");
        require(keccak256(user.encryptionPublicKey) == keccak256(hex"010203"), "Key mismatch");
        require(keccak256(bytes(user.nickname)) == keccak256("Alice"), "Nickname mismatch");
    }

    event UserRegistered(address indexed owner, string nasPeerId, bytes encryptionPublicKey, string nickname);
    event NasPeerIdUpdated(address indexed owner, string nasPeerId);
    event EncryptionPublicKeyUpdated(address indexed owner, bytes encryptionPublicKey);
    event NicknameUpdated(address indexed owner, string nickname);

    function testWriteOperationsEmitEvents() public {
        VM.expectEmit(true, false, false, true);
        emit UserRegistered(ALICE, "peer-alice", hex"010203", "Alice");
        registerAlice();
        VM.expectEmit(true, false, false, true);
        emit NasPeerIdUpdated(ALICE, "new-peer");
        VM.prank(ALICE);
        registry.updateNasPeerId("new-peer");
        VM.expectEmit(true, false, false, true);
        emit EncryptionPublicKeyUpdated(ALICE, hex"0405");
        VM.prank(ALICE);
        registry.updateEncryptionPublicKey(hex"0405");
        VM.expectEmit(true, false, false, true);
        emit NicknameUpdated(ALICE, "New name");
        VM.prank(ALICE);
        registry.updateNickname("New name");
    }

    function testRegisterAndRead() public {
        registerAlice();
        require(registry.isRegistered(ALICE), "Registration missing");
        assertAliceUnchanged();
    }

    function testDuplicateRegistrationRejected() public {
        registerAlice();
        VM.expectRevert(UserRegistry.AlreadyRegistered.selector);
        VM.prank(ALICE);
        registry.register("replacement", hex"04", "New name");
        assertAliceUnchanged();
    }

    function testUnknownUserRejected() public {
        require(!registry.isRegistered(ALICE), "Unexpected registration");
        VM.expectRevert(UserRegistry.NotRegistered.selector);
        registry.getUser(ALICE);
    }

    function testUnregisteredUpdatesRejected() public {
        VM.expectRevert(UserRegistry.NotRegistered.selector);
        registry.updateNasPeerId("peer");
        VM.expectRevert(UserRegistry.NotRegistered.selector);
        registry.updateEncryptionPublicKey(hex"01");
        VM.expectRevert(UserRegistry.NotRegistered.selector);
        registry.updateNickname("Name");
    }

    function testWalletCanUpdateOwnFields() public {
        registerAlice();
        VM.prank(ALICE);
        registry.updateNasPeerId("new-peer");
        VM.prank(ALICE);
        registry.updateEncryptionPublicKey(hex"0405");
        VM.prank(ALICE);
        registry.updateNickname("New name");
        UserRegistry.User memory user = registry.getUser(ALICE);
        require(keccak256(bytes(user.nasPeerId)) == keccak256("new-peer"), "PeerId unchanged");
        require(keccak256(user.encryptionPublicKey) == keccak256(hex"0405"), "Key unchanged");
        require(keccak256(bytes(user.nickname)) == keccak256("New name"), "Nickname unchanged");
    }

    function testUsersAreIsolatedAndMayShareNasAndNickname() public {
        registerAlice();
        VM.prank(BOB);
        registry.register("peer-alice", hex"aabb", "Alice");
        VM.prank(BOB);
        registry.updateNasPeerId("peer-bob");
        VM.prank(BOB);
        registry.updateEncryptionPublicKey(hex"ccdd");
        VM.prank(BOB);
        registry.updateNickname("Bob");
        assertAliceUnchanged();
    }

    function testEmptyRegistrationFieldsRejectedAtomically() public {
        VM.expectRevert(UserRegistry.InvalidNasPeerIdLength.selector);
        registry.register("", hex"01", "A");
        VM.expectRevert(UserRegistry.InvalidEncryptionPublicKeyLength.selector);
        registry.register("peer", hex"", "A");
        VM.expectRevert(UserRegistry.InvalidNicknameLength.selector);
        registry.register("peer", hex"01", "");
        require(!registry.isRegistered(address(this)), "Partial registration");
    }

    function testOversizeRegistrationFieldsRejected() public {
        VM.expectRevert(UserRegistry.InvalidNasPeerIdLength.selector);
        registry.register(string(new bytes(129)), hex"01", "A");
        VM.expectRevert(UserRegistry.InvalidEncryptionPublicKeyLength.selector);
        registry.register("peer", new bytes(257), "A");
        VM.expectRevert(UserRegistry.InvalidNicknameLength.selector);
        registry.register("peer", hex"01", string(new bytes(129)));
    }

    function testMaximumLengthsAccepted() public {
        registry.register(string(new bytes(128)), new bytes(256), string(new bytes(128)));
        UserRegistry.User memory user = registry.getUser(address(this));
        require(
            bytes(user.nasPeerId).length == 128 && user.encryptionPublicKey.length == 256
                && bytes(user.nickname).length == 128,
            "Boundary mismatch"
        );
    }

    function testInvalidUpdatesPreserveRecord() public {
        registerAlice();
        VM.expectRevert(UserRegistry.InvalidNasPeerIdLength.selector);
        VM.prank(ALICE);
        registry.updateNasPeerId("");
        VM.expectRevert(UserRegistry.InvalidEncryptionPublicKeyLength.selector);
        VM.prank(ALICE);
        registry.updateEncryptionPublicKey(hex"");
        VM.expectRevert(UserRegistry.InvalidNicknameLength.selector);
        VM.prank(ALICE);
        registry.updateNickname("");
        VM.expectRevert(UserRegistry.InvalidNasPeerIdLength.selector);
        VM.prank(ALICE);
        registry.updateNasPeerId(string(new bytes(129)));
        VM.expectRevert(UserRegistry.InvalidEncryptionPublicKeyLength.selector);
        VM.prank(ALICE);
        registry.updateEncryptionPublicKey(new bytes(257));
        VM.expectRevert(UserRegistry.InvalidNicknameLength.selector);
        VM.prank(ALICE);
        registry.updateNickname(string(new bytes(129)));
        assertAliceUnchanged();
    }

    function testFuzzOnlyCallerProfileChanges(address caller, bytes32 key) public {
        if (caller == ALICE || caller == address(0) || caller == address(VM)) return;
        registerAlice();
        VM.prank(caller);
        registry.register("peer-other", abi.encode(key), "Other");
        VM.prank(caller);
        registry.updateNickname("Changed");
        assertAliceUnchanged();
    }

    function testDeploymentRejectsWrongNetworkBeforeReadingKey() public {
        DeployUserRegistry deploy = new DeployUserRegistry();
        VM.chainId(1);
        VM.expectRevert(abi.encodeWithSelector(DeployUserRegistry.WrongChain.selector, uint256(1)));
        deploy.run();
    }
}
