// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;
import {PostAccess} from "../src/PostAccess.sol";

interface PostVm {
    function envUint(string calldata) external returns (uint256);
    function startBroadcast(uint256) external;
    function stopBroadcast() external;
}

contract DeployPostAccess {
    PostVm private constant vm = PostVm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function run() external returns (PostAccess deployed) {
        require(block.chainid == 84532, "Base Sepolia only");
        vm.startBroadcast(vm.envUint("DEPLOYER_PRIVATE_KEY"));
        deployed = new PostAccess();
        vm.stopBroadcast();
    }
}
