// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

import {UserRegistry} from "../src/UserRegistry.sol";

interface IDeploymentVm {
    function envUint(string calldata name) external returns (uint256);
    function startBroadcast(uint256 privateKey) external;
    function stopBroadcast() external;
}

contract DeployUserRegistry {
    IDeploymentVm private constant VM = IDeploymentVm(address(uint160(uint256(keccak256("hevm cheat code")))));

    error WrongChain(uint256 actualChainId);

    function run() external returns (UserRegistry registry) {
        if (block.chainid != 84532) revert WrongChain(block.chainid);
        uint256 deployerKey = VM.envUint("DEPLOYER_PRIVATE_KEY");
        VM.startBroadcast(deployerKey);
        registry = new UserRegistry();
        VM.stopBroadcast();
    }
}
