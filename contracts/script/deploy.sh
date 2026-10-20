#!/usr/bin/env bash
set +x
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ $# -gt 1 || ( $# -eq 1 && "$1" != "--broadcast" ) ]]; then
    echo "Usage: bash script/deploy.sh [--broadcast]" >&2
    exit 1
fi
if [[ ! -f .env.profile ]]; then
    echo "Missing contracts/.env.profile. Copy .env.example and fill in the deployment configuration." >&2
    exit 1
fi
set -a
source .env.profile
set +a
if [[ -z "${BASE_SEPOLIA_RPC_URL:-}" ]]; then
    echo "Set BASE_SEPOLIA_RPC_URL in contracts/.env.profile." >&2
    exit 1
fi
key_hex="${DEPLOYER_PRIVATE_KEY:-}"
case "$key_hex" in
    0x*|0X*) key_hex="${key_hex:2}" ;;
esac
if [[ ! "$key_hex" =~ ^[[:xdigit:]]{64}$ ]]; then
    echo "DEPLOYER_PRIVATE_KEY must contain exactly 64 hexadecimal characters, optionally prefixed with 0x." >&2
    exit 1
fi
if [[ "$key_hex" =~ ^0{64}$ ]]; then
    echo "DEPLOYER_PRIVATE_KEY must not be zero." >&2
    exit 1
fi
export DEPLOYER_PRIVATE_KEY="0x${key_hex}"
unset key_hex
# Named RPC configuration and envUint keep credentials out of command-line arguments.
forge script script/DeployUserRegistry.s.sol:DeployUserRegistry --rpc-url base_sepolia "$@"
