// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

/// @notice Wallet-owned public profiles. No private keys or off-chain profile data are stored.
/// @dev PeerIds and encryption keys are bounded opaque values, not validated cryptographic identities.
contract UserRegistry {
    uint256 public constant MAX_NAS_PEER_ID_BYTES = 128;
    uint256 public constant MAX_ENCRYPTION_PUBLIC_KEY_BYTES = 256;
    uint256 public constant MAX_NICKNAME_BYTES = 128;

    struct User {
        string nasPeerId;
        bytes encryptionPublicKey;
        string nickname;
    }

    mapping(address owner => User) private users;

    error AlreadyRegistered();
    error NotRegistered();
    error InvalidNasPeerIdLength();
    error InvalidEncryptionPublicKeyLength();
    error InvalidNicknameLength();

    event UserRegistered(address indexed owner, string nasPeerId, bytes encryptionPublicKey, string nickname);
    event NasPeerIdUpdated(address indexed owner, string nasPeerId);
    event EncryptionPublicKeyUpdated(address indexed owner, bytes encryptionPublicKey);
    event NicknameUpdated(address indexed owner, string nickname);

    function register(string calldata nasPeerId, bytes calldata encryptionPublicKey, string calldata nickname)
        external
    {
        if (isRegistered(msg.sender)) revert AlreadyRegistered();
        validateNasPeerId(nasPeerId);
        validateEncryptionPublicKey(encryptionPublicKey);
        validateNickname(nickname);
        users[msg.sender] = User({nasPeerId: nasPeerId, encryptionPublicKey: encryptionPublicKey, nickname: nickname});
        emit UserRegistered(msg.sender, nasPeerId, encryptionPublicKey, nickname);
    }

    function updateNasPeerId(string calldata nasPeerId) external {
        requireRegistered();
        validateNasPeerId(nasPeerId);
        users[msg.sender].nasPeerId = nasPeerId;
        emit NasPeerIdUpdated(msg.sender, nasPeerId);
    }

    function updateEncryptionPublicKey(bytes calldata encryptionPublicKey) external {
        requireRegistered();
        validateEncryptionPublicKey(encryptionPublicKey);
        users[msg.sender].encryptionPublicKey = encryptionPublicKey;
        emit EncryptionPublicKeyUpdated(msg.sender, encryptionPublicKey);
    }

    function updateNickname(string calldata nickname) external {
        requireRegistered();
        validateNickname(nickname);
        users[msg.sender].nickname = nickname;
        emit NicknameUpdated(msg.sender, nickname);
    }

    function getUser(address owner) external view returns (User memory) {
        if (!isRegistered(owner)) revert NotRegistered();
        return users[owner];
    }

    function isRegistered(address owner) public view returns (bool) {
        return bytes(users[owner].nasPeerId).length != 0;
    }

    function requireRegistered() private view {
        if (!isRegistered(msg.sender)) revert NotRegistered();
    }

    function validateNasPeerId(string calldata value) private pure {
        if (bytes(value).length == 0 || bytes(value).length > MAX_NAS_PEER_ID_BYTES) revert InvalidNasPeerIdLength();
    }

    function validateEncryptionPublicKey(bytes calldata value) private pure {
        if (value.length == 0 || value.length > MAX_ENCRYPTION_PUBLIC_KEY_BYTES) {
            revert InvalidEncryptionPublicKeyLength();
        }
    }

    function validateNickname(string calldata value) private pure {
        if (bytes(value).length == 0 || bytes(value).length > MAX_NICKNAME_BYTES) revert InvalidNicknameLength();
    }
}
