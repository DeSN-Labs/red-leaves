import { activateWallet, chooseWallet, type Provider, type WalletChoice } from "./wallet-provider.ts";
export { availableWallets, preferredWalletId, getWalletProvider } from "./wallet-provider.ts";
export type { Provider, WalletChoice } from "./wallet-provider.ts";

export const apiBaseUrl = (import.meta.env?.VITE_API_BASE_URL ?? "http://127.0.0.1:3000").replace(/\/+$/, "");

let authenticated = false;
let expiresAt = 0;
let revision = 0;

type NasSession = { address: string; chain_id: number; expires_at: number };
let restoration: Promise<string | null> | null = null;
const webHeaders = { "X-Red-Leaves-Client": "web" };
class AuthenticationError extends Error {
  readonly status: number;
  constructor(status: number) {
    super(`NAS authentication failed (${status}). Check the node connection or sign in again.`);
    this.status = status;
  }
}
function validSession(value: unknown): value is NasSession {
  if (!value || typeof value !== "object") return false;
  const session = value as Partial<NasSession>;
  return typeof session.address === "string" && /^0x[0-9a-f]{40}$/i.test(session.address)
    && session.chain_id === 84532 && Number.isSafeInteger(session.expires_at)
    && session.expires_at! > Date.now() / 1000;
}
function ensureAttempt(attempt: number) {
  if (attempt !== revision) throw new Error("Wallet state changed. Please sign in again.");
}
async function walletMatches(provider: Provider, address: string) {
  const [accounts, chain] = await Promise.all([
    provider.request({ method: "eth_accounts" }), provider.request({ method: "eth_chainId" }),
  ]);
  return Array.isArray(accounts) && typeof accounts[0] === "string"
    && accounts[0].toLowerCase() === address.toLowerCase() && chain === "0x14a34";
}
// Only the NAS can read the HttpOnly cookie; restoration never requests a signature.
export function restoreWallet(): Promise<string | null> {
  if (restoration) return restoration;
  restoration = restoreSession().finally(() => { restoration = null; });
  return restoration;
}
async function restoreSession(): Promise<string | null> {
  const attempt = ++revision;
  let session: NasSession;
  try { session = await api<NasSession>("me"); }
  catch (cause) {
    ensureAttempt(attempt);
    if (cause instanceof AuthenticationError && cause.status === 401) return null;
    throw cause;
  }
  ensureAttempt(attempt);
  const choice = await chooseWallet();
  ensureAttempt(attempt);
  const provider = choice.provider;
  const matches = validSession(session) && await walletMatches(provider, session.address);
  ensureAttempt(attempt);
  if (!matches) { await logoutWallet(); return null; }
  activateWallet(choice);
  authenticated = true; expiresAt = session.expires_at;
  return session.address;
}

async function api<T>(path: string, body?: unknown): Promise<T> {
  const response = await fetch(`${apiBaseUrl}/api/auth/${path}`, {
    method: body === undefined ? "GET" : "POST",
    credentials: "include",
    headers: { "Content-Type": "application/json", ...webHeaders },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    signal: AbortSignal.timeout(15000),
  });
  if (!response.ok) throw new AuthenticationError(response.status);
  return response.status === 204 ? undefined as T : response.json();
}
export const sessionExpiresAt = () => expiresAt;
export async function nasApi<T>(path: string, body?: unknown): Promise<T> {
  const wasAuthenticated = authenticated, currentRevision = revision;
  if (!wasAuthenticated) throw new Error("Sign in with your wallet to use your NAS.");
  const response = await fetch(`${apiBaseUrl}/api/${path}`, {
    method: body === undefined ? "GET" : "POST", credentials: "include",
    headers: { "Content-Type": "application/json", ...webHeaders },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }), signal: AbortSignal.timeout(65000),
  });
  if (wasAuthenticated !== authenticated || currentRevision !== revision) throw new Error("Wallet state changed. Please sign in again.");
  if (!response.ok) {
    const value = await response.json().catch(() => null) as { error?: string } | null;
    throw new Error(value?.error ?? `NAS request failed (${response.status}).`);
  }
  const result = await response.json() as T;
  if (wasAuthenticated !== authenticated || currentRevision !== revision) throw new Error("Wallet state changed. Please sign in again.");
  return result;
}
let mediaTransfers = 0;
const mediaWaiters: (() => void)[] = [];
async function nasTransfer<T>(path: string, file?: File, signal?: AbortSignal): Promise<T> {
  const wasAuthenticated = authenticated, currentRevision = revision;
  if (!wasAuthenticated) throw new Error("Sign in with your wallet to use your NAS.");
  // Match the NAS transfer bound so a visible gallery does not flood it with requests.
  if (mediaTransfers >= 2) await new Promise<void>(resolve => mediaWaiters.push(resolve));
  else mediaTransfers++;
  try {
    signal?.throwIfAborted();
    if (wasAuthenticated !== authenticated || currentRevision !== revision) throw new Error("Wallet state changed. Please sign in again.");
    const response = await fetch(`${apiBaseUrl}/api/${path}`, {
      method: file ? "POST" : "GET", credentials: "include",
      headers: { ...webHeaders, ...(file ? { "Content-Type": file.type } : {}) },
      ...(file ? { body: file } : {}), signal: signal ? AbortSignal.any([signal, AbortSignal.timeout(65000)]) : AbortSignal.timeout(65000),
    });
    if (wasAuthenticated !== authenticated || currentRevision !== revision) throw new Error("Wallet state changed. Please sign in again.");
    if (!response.ok) {
      const error = await response.json().catch(() => null) as { error?: string } | null;
      throw new Error(error?.error ?? `NAS media request failed (${response.status}).`);
    }
    const result = file ? await response.json() : await response.blob();
    if (wasAuthenticated !== authenticated || currentRevision !== revision) throw new Error("Wallet state changed. Please sign in again.");
    return result as T;
  } finally {
    const next = mediaWaiters.shift();
    if (next) next(); else mediaTransfers--;
  }
}
export const nasUpload = <T>(file: File) => nasTransfer<T>("media", file);
export const nasBlob = (path: string, signal?: AbortSignal) => nasTransfer<Blob>(path, undefined, signal);
export async function watchNasChanges(onChange: () => void, signal: AbortSignal): Promise<void> {
  const wasAuthenticated = authenticated, currentRevision = revision;
  if (!wasAuthenticated) throw new Error("Sign in with your wallet to receive updates.");
  while (!signal.aborted) {
    if (authenticated !== wasAuthenticated || revision !== currentRevision) throw new Error("Wallet state changed. Please sign in again.");
    try {
      const response = await fetch(`${apiBaseUrl}/api/social/events`, {
        credentials: "include", headers: { ...webHeaders, Accept: "text/event-stream" },
        signal: AbortSignal.any([signal, AbortSignal.timeout(90000)]),
      });
      if (response.status === 401) throw new Error("The NAS session expired. Sign in again.");
      if (!response.ok || !response.body) throw new Error("Live NAS updates are unavailable.");
      const reader = response.body.getReader(), decoder = new TextDecoder();
      let buffer = "";
      try {
        for (;;) {
          if (signal.aborted) return;
          const result = await reader.read();
          if (signal.aborted) return;
          if (authenticated !== wasAuthenticated || revision !== currentRevision) throw new Error("Wallet state changed. Please sign in again.");
          if (result.done) break;
          buffer += decoder.decode(result.value, { stream: true }).replace(/\r/g, "");
          if (buffer.length > 65536) throw new Error("Invalid NAS event stream.");
          let boundary: number;
          while ((boundary = buffer.indexOf("\n\n")) >= 0) {
            if (signal.aborted) return;
            if (authenticated !== wasAuthenticated || revision !== currentRevision) throw new Error("Wallet state changed. Please sign in again.");
            const event = buffer.slice(0, boundary); buffer = buffer.slice(boundary + 2);
            if (/^event:\s*(update|sync)\s*$/m.test(event)) onChange();
          }
        }
      } finally { await reader.cancel().catch(() => {}); reader.releaseLock(); }
    } catch (cause) {
      if (signal.aborted) return;
      if (authenticated !== wasAuthenticated || revision !== currentRevision || (cause instanceof Error && cause.message.includes("session expired"))) throw cause;
    }
    await new Promise<void>(resolve => {
      const finish = () => { clearTimeout(timer); signal.removeEventListener("abort", finish); resolve(); };
      const timer = setTimeout(finish, 3000); signal.addEventListener("abort", finish, { once: true });
    });
  }
}
export async function getNasInfo() {
  const wasAuthenticated = authenticated, currentRevision = revision;
  if (!wasAuthenticated) throw new Error("Sign in with your wallet to connect to the NAS.");
  const response = await fetch(`${apiBaseUrl}/api/nas/info`, {
    credentials: "include", headers: { ...webHeaders }, signal: AbortSignal.timeout(15000),
  });
  if (response.status === 404) throw new Error("The NAS identity endpoint is unavailable. Restart the NAS server with the latest build, then refresh profile status.");
  if (!response.ok) throw new Error(`NAS identity request failed (${response.status}). Check your login session.`);
  if (authenticated !== wasAuthenticated || revision !== currentRevision) throw new Error("Wallet state changed. Please sign in again.");
  const result = await response.json() as { node_type: string; peer_id: string; p2p_running: boolean };
  if (result.node_type !== "nas") throw new Error("Profile registration requires a NAS node. Restart an outdated NAS server or connect to a NAS node.");
  if (typeof result.peer_id !== "string" || !result.peer_id || new TextEncoder().encode(result.peer_id).length > 128) throw new Error("The NAS returned an invalid PeerId.");
  return result;
}
export async function checkNasSession() {
  const wasAuthenticated = authenticated, currentRevision = revision;
  if (!wasAuthenticated) throw new Error("Sign in with your wallet to connect to the NAS.");
  const session = await api<{ address: string; chain_id: number; expires_at: number }>("me");
  if (authenticated !== wasAuthenticated || revision !== currentRevision) throw new Error("Wallet state changed. Please sign in again.");
  return session;
}
export async function logoutWallet() {
  revision++;
  authenticated = false;
  expiresAt = 0;
  await api("logout", {});
}
export async function connectWallet(selected?: WalletChoice) {
  const attempt = ++revision;
  const choice = await chooseWallet(selected);
  ensureAttempt(attempt);
  const provider = choice.provider;
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
  const session = await api<NasSession>("verify", {
    challenge_id: challenge.challenge_id, verifier: challenge.verifier, signature,
  });
  if (attempt !== revision) {
    await api("logout", {}).catch(() => {});
    throw new Error("Wallet state changed. Please sign in again.");
  }
  if (!validSession(session) || session.address.toLowerCase() !== address.toLowerCase()) {
    await api("logout", {}).catch(() => {});
    throw new Error("The NAS returned an invalid login session. Nothing was saved.");
  }
  const stillMatches = await walletMatches(provider, address);
  if (!stillMatches || attempt !== revision) {
    await api("logout", {}).catch(() => {});
    throw new Error("Wallet state changed. Please sign in again.");
  }
  activateWallet(choice);
  authenticated = true;
  expiresAt = session.expires_at;
  return session.address;
}
export const shortAddress = (address: string) => `${address.slice(0, 6)}…${address.slice(-4)}`;
