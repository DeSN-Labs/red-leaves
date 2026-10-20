export interface Provider {
  request(args: { method: string; params?: unknown[] }): Promise<unknown>;
  on?(event: string, callback: (...args: unknown[]) => void): void;
  removeListener?(event: string, callback: (...args: unknown[]) => void): void;
}
declare global { interface Window { ethereum?: Provider } }

export const apiBaseUrl = (import.meta.env?.VITE_API_BASE_URL ?? "http://127.0.0.1:3000").replace(/\/+$/, "");

let token: string | null = null;
let expiresAt = 0;
let revision = 0;

async function api<T>(path: string, body?: unknown, bearer?: string): Promise<T> {
  const response = await fetch(`${apiBaseUrl}/api/auth/${path}`, {
    method: body === undefined ? "GET" : "POST",
    credentials: "omit",
    headers: { "Content-Type": "application/json", ...(bearer ? { Authorization: `Bearer ${bearer}` } : {}) },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    signal: AbortSignal.timeout(15000),
  });
  if (!response.ok) throw new Error(`NAS authentication failed (${response.status}). Check the node connection or sign in again.`);
  return response.status === 204 ? undefined as T : response.json();
}
export const sessionExpiresAt = () => expiresAt;
export async function getNasInfo() {
  const currentToken = token;
  if (!currentToken) throw new Error("Sign in with your wallet to connect to the NAS.");
  const response = await fetch(`${apiBaseUrl}/api/nas/info`, {
    credentials: "omit", headers: { Authorization: `Bearer ${currentToken}` }, signal: AbortSignal.timeout(15000),
  });
  if (response.status === 404) throw new Error("The NAS identity endpoint is unavailable. Restart the NAS server with the latest build, then refresh profile status.");
  if (!response.ok) throw new Error(`NAS identity request failed (${response.status}). Check your login session.`);
  if (token !== currentToken) throw new Error("Wallet state changed. Please sign in again.");
  const result = await response.json() as { node_type: string; peer_id: string; p2p_running: boolean };
  if (result.node_type !== "nas") throw new Error("Profile registration requires a NAS node. Restart an outdated NAS server or connect to a NAS node.");
  if (typeof result.peer_id !== "string" || !result.peer_id || new TextEncoder().encode(result.peer_id).length > 128) throw new Error("The NAS returned an invalid PeerId.");
  return result;
}
export async function checkNasSession() {
  const currentToken = token;
  if (!currentToken) throw new Error("Sign in with your wallet to connect to the NAS.");
  const session = await api<{ address: string; chain_id: number; expires_at: number }>("me", undefined, currentToken);
  if (token !== currentToken) throw new Error("Wallet state changed. Please sign in again.");
  return session;
}
export async function logoutWallet() {
  revision++;
  const previous = token;
  token = null;
  expiresAt = 0;
  if (previous) await api("logout", {}, previous);
}
export async function connectWallet() {
  const provider = window.ethereum;
  if (!provider) throw new Error("No wallet extension found. Install a compatible wallet such as MetaMask, or try the demo.");
  const attempt = ++revision;
  const accounts = await provider.request({ method: "eth_requestAccounts" });
  if (!Array.isArray(accounts) || typeof accounts[0] !== "string" || !/^0x[a-fA-F0-9]{40}$/.test(accounts[0])) {
    throw new Error("The wallet did not return a valid address. Please try again.");
  }
  const address = accounts[0];
  const chain = await provider.request({ method: "eth_chainId" });
  if (chain !== "0x14a34") throw new Error("Please switch your wallet to Base Sepolia first.");
  const challenge = await api<{ challenge_id: string; verifier: string; message: string }>("challenge", { address });
  if (attempt !== revision) throw new Error("Wallet state changed. Please sign in again.");
  const encoded = `0x${Array.from(new TextEncoder().encode(challenge.message), b => b.toString(16).padStart(2, "0")).join("")}`;
  const signature = await provider.request({ method: "personal_sign", params: [encoded, address] });
  if (attempt !== revision) throw new Error("Wallet state changed. Please sign in again.");
  const currentAccounts = await provider.request({ method: "eth_accounts" });
  const currentChain = await provider.request({ method: "eth_chainId" });
  if (!Array.isArray(currentAccounts) || typeof currentAccounts[0] !== "string" || currentAccounts[0].toLowerCase() !== address.toLowerCase() || currentChain !== chain || attempt !== revision) {
    throw new Error("Wallet state changed. Please sign in again.");
  }
  if (typeof signature !== "string") throw new Error("The wallet returned an invalid signature.");
  const session = await api<{ token: string; address: string; chain_id: number; expires_at: number }>("verify", {
    challenge_id: challenge.challenge_id, verifier: challenge.verifier, signature,
  });
  if (attempt !== revision) {
    await api("logout", {}, session.token);
    throw new Error("Wallet state changed. Please sign in again.");
  }
  token = session.token;
  expiresAt = session.expires_at;
  return session.address;
}
export const shortAddress = (address: string) => `${address.slice(0, 6)}…${address.slice(-4)}`;
