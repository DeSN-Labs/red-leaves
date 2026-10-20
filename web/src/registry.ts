import {
  createPublicClient, createWalletClient, custom, getAddress, isAddress,
  type Address, type EIP1193Provider, type Hex,
} from "viem";
import { baseSepolia } from "viem/chains";
import type { Provider } from "./wallet.ts";

export const registryAddress = import.meta.env?.VITE_USER_REGISTRY_ADDRESS ?? "";
export const registryAbi = [
  { type: "function", name: "isRegistered", stateMutability: "view", inputs: [{ name: "owner", type: "address" }], outputs: [{ type: "bool" }] },
  { type: "function", name: "getUser", stateMutability: "view", inputs: [{ name: "owner", type: "address" }], outputs: [{ type: "tuple", components: [{ name: "nasPeerId", type: "string" }, { name: "encryptionPublicKey", type: "bytes" }, { name: "nickname", type: "string" }] }] },
  { type: "function", name: "register", stateMutability: "nonpayable", inputs: [{ name: "nasPeerId", type: "string" }, { name: "encryptionPublicKey", type: "bytes" }, { name: "nickname", type: "string" }], outputs: [] },
] as const;
export type Registration = { nasPeerId: string; encryptionPublicKey: Hex; nickname: string };
export type PendingRegistration = Registration & { hash: Hex };

export function validateRegistration(input: Registration) {
  const size = (value: string) => new TextEncoder().encode(value).length;
  if (!input.nasPeerId || size(input.nasPeerId) > 128) throw new Error("NAS PeerId must contain 1 to 128 bytes.");
  if (!/^0x(?:[0-9a-fA-F]{2}){1,256}$/.test(input.encryptionPublicKey)) throw new Error("Enter a hex public key of 1 to 256 bytes with a 0x prefix.");
  if (!input.nickname.trim() || size(input.nickname) > 128) throw new Error("Nickname must contain 1 to 128 UTF-8 bytes and must not be blank.");
}

export function createRegistry(provider: Provider, contract: string, owner: string, storage: Storage) {
  if (!isAddress(contract) || /^0x0{40}$/i.test(contract)) throw new Error("Set VITE_USER_REGISTRY_ADDRESS in web/.env.profile to the deployed registry address.");
  const address = getAddress(contract);
  const account = getAddress(owner);
  const transport = custom(provider as EIP1193Provider, { retryCount: 0 });
  const client = createPublicClient({ chain: baseSepolia, transport });
  const wallet = createWalletClient({ account, chain: baseSepolia, transport });
  const storageKey = `red-leaves:pending-registration:84532:${address.toLowerCase()}:${account.toLowerCase()}`;
  const confirmations = 2;

  async function scope() {
    if (await client.getChainId() !== baseSepolia.id) throw new Error("Please switch your wallet to Base Sepolia.");
    const accounts = await wallet.getAddresses();
    if (!accounts.some(value => value.toLowerCase() === account.toLowerCase())) throw new Error("Wallet account changed. Sign in again.");
    const code = await client.getBytecode({ address });
    if (!code || code === "0x") throw new Error("No registry contract exists at the configured address on Base Sepolia.");
  }
  function pending(): PendingRegistration | null {
    const raw = storage.getItem(storageKey);
    if (!raw) return null;
    const value = JSON.parse(raw) as PendingRegistration;
    validateRegistration(value);
    if (!/^0x[0-9a-fA-F]{64}$/.test(value.hash)) throw new Error("Invalid pending registration. Check the transaction history before retrying.");
    return value;
  }
  async function read(blockNumber?: bigint): Promise<Registration | null> {
    await scope();
    const block = blockNumber ?? await client.getBlockNumber({ cacheTime: 0 });
    const exists = await client.readContract({ address, abi: registryAbi, functionName: "isRegistered", args: [account], blockNumber: block });
    if (!exists) return null;
    return client.readContract({ address, abi: registryAbi, functionName: "getUser", args: [account], blockNumber: block });
  }
  async function confirm(value: PendingRegistration, onHash: (hash: Hex) => void) {
    await scope();
    let canceled = false;
    const receipt = await client.waitForTransactionReceipt({
      hash: value.hash, confirmations, timeout: 120_000,
      onReplaced(replacement) {
        canceled = replacement.reason === "cancelled";
        value = { ...value, hash: replacement.transactionReceipt.transactionHash };
        storage.setItem(storageKey, JSON.stringify(value));
        onHash(value.hash);
      },
    });
    if (receipt.status !== "success" || canceled) {
      storage.removeItem(storageKey);
      throw new Error("Registration transaction reverted or was canceled. Your NAS login is still active.");
    }
    const result = await read(receipt.blockNumber);
    if (!result || result.nasPeerId !== value.nasPeerId || result.encryptionPublicKey.toLowerCase() !== value.encryptionPublicKey.toLowerCase() || result.nickname !== value.nickname) {
      throw new Error("The confirmed transaction does not match the requested registration. Refresh the on-chain record.");
    }
    storage.removeItem(storageKey);
    return result;
  }
  return {
    read, pending,
    async resume(onHash: (hash: Hex) => void) {
      const value = pending();
      if (!value) throw new Error("No pending registration transaction was found.");
      onHash(value.hash);
      return confirm(value, onHash);
    },
    async register(input: Registration, onHash: (hash: Hex) => void, isActive: () => boolean = () => true) {
      validateRegistration(input);
      if (pending()) throw new Error("Resume the pending transaction before submitting another registration.");
      if (await read()) throw new Error("This wallet is already registered. Refresh its on-chain record.");
      // Ensure pending-transaction storage is available before requesting a paid transaction.
      storage.setItem(`${storageKey}:probe`, JSON.stringify({ ...input, hash: `0x${"0".repeat(64)}` }));
      storage.removeItem(`${storageKey}:probe`);
      const { request } = await client.simulateContract({ address, abi: registryAbi, functionName: "register", args: [input.nasPeerId, input.encryptionPublicKey, input.nickname], account });
      await scope();
      if (!isActive()) throw new Error("The login session changed. Registration was not submitted.");
      const hash = await wallet.writeContract(request);
      const value = { ...input, hash };
      storage.setItem(storageKey, JSON.stringify(value));
      onHash(hash);
      return confirm(value, onHash);
    },
  };
}
