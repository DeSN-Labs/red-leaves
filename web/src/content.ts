import { createPublicClient, createWalletClient, custom, bytesToHex, hexToBytes, keccak256, type Address, type EIP1193Provider, type Hex } from "viem";
import { baseSepolia } from "viem/chains";
import { getNasInfo, checkNasSession, nasApi, getWalletProvider } from "./wallet.ts";
import { registryAddress, registryAbi } from "./registry.ts";

export type Distribution = "broadcast" | "following" | "followers" | "friends" | "directed";
export type Media = { id: string; mime: string; size: number; digest: string };
export type Draft = { title: string; body: string; distribution: Distribution; recipients: number[][]; media?: Media[] };
export type SocialAction = { action: "publish"; post: Draft } | { action: "follow"; target: number[]; enabled: boolean } | { action: "like"; post_id: number[]; enabled: boolean };
export type Preferences = { broadcasts: boolean; following: boolean; followers: boolean; direct: boolean };
export type PostSummary = { id: string; author: string; title: string; distribution: string; created_at: number; published: boolean; preview?: Draft | null; delivery?: Record<string, number> };
export type PostStatus = { id: string; status: "saved_pending_network"; author: Address };
export type ActionStatus = PostStatus | { status: "saved" };
export type PostDetail = { id: string; intent: { issued_at: number; action: { action: "publish"; post: Draft } }; signature: string };
export type Relationships = { following: string[]; followers: string[] };
export function audienceWallets(distribution: Distribution, relationships: Relationships, selected: string[] = []): string[] {
  const following = relationships.following.map(value => value.toLowerCase()), followers = relationships.followers.map(value => value.toLowerCase());
  const values = distribution === "broadcast" ? [] : distribution === "following" ? following : distribution === "followers" ? followers : distribution === "friends" ? following.filter(value => followers.includes(value)) : selected.map(value => value.toLowerCase());
  return [...new Set(values)].sort();
}

export function addressBytes(value: string) {
  if (!/^0x[0-9a-fA-F]{40}$/.test(value) || /^0x0{40}$/i.test(value)) throw new Error("Enter a valid wallet address.");
  return Array.from(hexToBytes(value as Hex));
}
// The protocol's deliberately restricted RFC 8949 deterministic CBOR profile.
// This independently reconstructs what the user reviewed, instead of trusting a NAS digest.
type Cbor = null | boolean | number | string | Uint8Array | Cbor[] | { [key: string]: Cbor };
export function encodeCanonical(value: Cbor): Uint8Array {
  const out: number[] = [];
  function head(major: number, count: number) {
    if (!Number.isSafeInteger(count) || count < 0) throw new Error("Invalid canonical integer.");
    if (count < 24) out.push(major * 32 + count);
    else if (count <= 255) out.push(major * 32 + 24, count);
    else if (count <= 65535) out.push(major * 32 + 25, count >> 8, count & 255);
    else {
      const bytes = new Uint8Array(count <= 0xffffffff ? 4 : 8), view = new DataView(bytes.buffer);
      if (bytes.length === 4) view.setUint32(0, count); else view.setBigUint64(0, BigInt(count));
      out.push(major * 32 + (bytes.length === 4 ? 26 : 27), ...bytes);
    }
  }
  function write(item: Cbor) {
    if (item === null) out.push(246);
    else if (typeof item === "boolean") out.push(item ? 245 : 244);
    else if (typeof item === "number") head(0, item);
    else if (typeof item === "string") { const bytes = new TextEncoder().encode(item); head(3, bytes.length); out.push(...bytes); }
    else if (item instanceof Uint8Array) { head(2, item.length); out.push(...item); }
    else if (Array.isArray(item)) { head(4, item.length); item.forEach(write); }
    else {
      const keys = Object.keys(item).map(key => ({ key, encoded: encodeCanonical(key) }));
      keys.sort((a, b) => { for (let i = 0; i < Math.min(a.encoded.length, b.encoded.length); i++) if (a.encoded[i] !== b.encoded[i]) return a.encoded[i] - b.encoded[i]; return a.encoded.length - b.encoded.length; });
      head(5, keys.length); keys.forEach(({ key }) => { write(key); write(item[key]); });
    }
  }
  write(value); return new Uint8Array(out);
}
function clients(owner: string) {
  const transport = custom(getWalletProvider() as EIP1193Provider, { retryCount: 0 });
  return { wallet: createWalletClient({ account: owner as Address, chain: baseSepolia, transport }), public: createPublicClient({ chain: baseSepolia, transport }) };
}
async function checkWallet(owner: string) {
  const session = await checkNasSession();
  if (session.address.toLowerCase() !== owner.toLowerCase()) throw new Error("The NAS session belongs to another wallet. Sign in again.");
  const client = clients(owner);
  const addresses = await client.wallet.getAddresses();
  if (addresses[0]?.toLowerCase() !== owner.toLowerCase() || await client.public.getChainId() !== 84532) throw new Error("Wallet account or network changed. Sign in again on Base Sepolia.");
  return client;
}

export async function submitAction(owner: string, action: SocialAction): Promise<ActionStatus> {
  const prepared = await nasApi<{ intent: { scope: { protocol_family: string; network_version: number; chain_id: number; registry_contract: number[] }; actor: number[]; nas_peer_id: string; nonce: number[]; issued_at: number; expires_at: number; action: SocialAction }; payload: Hex; payload_hash: Hex }>("social/prepare", { action });
  const { intent } = prepared;
  const nas = await getNasInfo();
  if (!actionsMatch(intent.action, action)) throw new Error("The NAS changed the action. Nothing was signed.");
  if (bytesToHex(new Uint8Array(intent.actor)).toLowerCase() !== owner.toLowerCase() || intent.nas_peer_id !== nas.peer_id || intent.scope.chain_id !== 84532 || bytesToHex(new Uint8Array(intent.scope.registry_contract)).toLowerCase() !== registryAddress.toLowerCase() || intent.scope.protocol_family !== "red-leaves" || intent.scope.network_version !== 1 || intent.expires_at <= Date.now() / 1000 || intent.expires_at - intent.issued_at > 900) throw new Error("The NAS returned an invalid signing scope.");
  const typedAction = intent.action.action === "publish" ? { action: "publish", post: { ...intent.action.post, recipients: intent.action.post.recipients.map(value => new Uint8Array(value)) } } : intent.action.action === "follow" ? { ...intent.action, target: new Uint8Array(intent.action.target) } : { ...intent.action, post_id: new Uint8Array(intent.action.post_id) };
  const encoded = encodeCanonical({ ...intent, scope: { ...intent.scope, registry_contract: new Uint8Array(intent.scope.registry_contract) }, actor: new Uint8Array(intent.actor), nonce: new Uint8Array(intent.nonce), action: typedAction } as Cbor);
  if (bytesToHex(encoded) !== prepared.payload || keccak256(encoded) !== prepared.payload_hash) throw new Error("The NAS payload does not match the reviewed action.");
  const client = await checkWallet(owner);
  const signature = await client.wallet.signTypedData({ domain: { name: "RedLeaves", version: "1", chainId: 84532, verifyingContract: registryAddress as Address }, types: { SocialAction: [{ name: "payloadHash", type: "bytes32" }] }, primaryType: "SocialAction", message: { payloadHash: prepared.payload_hash } });
  await checkWallet(owner);
  return nasApi<ActionStatus>("social/commit", { payload: prepared.payload, signature });
}
export function actionsMatch(actual: SocialAction, reviewed: SocialAction) {
  // Map ordering may differ between JSON serializers, including nested media descriptors.
  return bytesToHex(encodeCanonical(actual as unknown as Cbor)) === bytesToHex(encodeCanonical(reviewed as unknown as Cbor));
}
export async function updateNasKey(owner: string, publicKey: Hex, onHash: (hash: Hex) => void) {
  const client = await checkWallet(owner);
  const request = await client.public.simulateContract({ address: registryAddress as Address, abi: registryAbi, functionName: "updateEncryptionPublicKey", args: [publicKey], account: owner as Address });
  const hash = await client.wallet.writeContract(request.request); onHash(hash);
  const receipt = await client.public.waitForTransactionReceipt({ hash, confirmations: 3, timeout: 120000 });
  if (receipt.status !== "success") throw new Error("Encryption key update reverted.");
}
