export interface Provider {
  request(args: { method: string; params?: unknown[] }): Promise<unknown>;
  on?(event: string, callback: (...args: unknown[]) => void): void;
  removeListener?(event: string, callback: (...args: unknown[]) => void): void;
}
export type WalletChoice = { id: string; name: string; provider: Provider };
type InjectedProvider = Provider & { providers?: InjectedProvider[]; isMetaMask?: boolean; isCoinbaseWallet?: boolean; isRabby?: boolean; isBraveWallet?: boolean; isOkxWallet?: boolean };
declare global { interface Window { ethereum?: InjectedProvider } }
const preferenceKey = "red-leaves:wallet-provider";
const states = new WeakMap<Window, { choices: Map<string, WalletChoice>; active?: WalletChoice }>();
function state() {
  let current = states.get(window);
  if (!current) {
    current = { choices: new Map() };
    states.set(window, current);
    const choices = current.choices;
    window.addEventListener?.("eip6963:announceProvider", ((event: CustomEvent) => {
      const detail = event.detail;
      if (typeof detail?.provider?.request !== "function" || typeof detail?.info?.rdns !== "string"
        || !/^[a-zA-Z0-9-]+(?:\.[a-zA-Z0-9-]+)+$/.test(detail.info.rdns)
        || detail.info.rdns.length > 253 || typeof detail.info.name !== "string" || detail.info.name.length > 100) return;
      const id = `eip6963:${detail.info.rdns}`;
      // Conflicting announcements must not silently replace a selected wallet.
      if (!choices.has(id) && choices.size < 32) choices.set(id, { id, name: detail.info.name, provider: detail.provider });
    }) as EventListener);
  }
  return current;
}
function legacyChoices(): WalletChoice[] {
  const injected = window.ethereum;
  if (!injected) return [];
  return [...new Set(injected.providers?.length ? injected.providers : [injected])].flatMap(provider => {
    const brand = provider.isRabby ? "Rabby" : provider.isCoinbaseWallet ? "Coinbase Wallet" : provider.isBraveWallet ? "Brave Wallet" : provider.isOkxWallet ? "OKX Wallet" : provider.isMetaMask ? "MetaMask" : null;
    if (!brand && injected.providers && injected.providers.length > 1) return [];
    return [{ id: `legacy:${brand ?? "injected"}`, name: brand ?? "Browser wallet", provider }];
  });
}
export async function availableWallets(): Promise<WalletChoice[]> {
  const current = state();
  if (typeof window.dispatchEvent === "function") {
    window.dispatchEvent(new Event("eip6963:requestProvider"));
    // Allow extensions injected after the application to announce themselves.
    await new Promise(resolve => setTimeout(resolve, 200));
  }
  return current.choices.size ? [...current.choices.values()] : legacyChoices();
}
export function preferredWalletId(): string | null {
  try { return window.localStorage?.getItem(preferenceKey) ?? null; } catch { return null; }
}
export async function chooseWallet(selected?: WalletChoice): Promise<WalletChoice> {
  let choices = await availableWallets();
  const id = selected?.id ?? preferredWalletId();
  let choice = id ? choices.find(value => value.id === id && (!selected || value.provider === selected.provider)) : choices.length === 1 ? choices[0] : undefined;
  // During refresh, wait briefly for the remembered extension rather than using a different wallet.
  if (!choice && id && typeof window.dispatchEvent === "function") {
    await new Promise(resolve => setTimeout(resolve, 800));
    choices = await availableWallets();
    choice = choices.find(value => value.id === id && (!selected || value.provider === selected.provider));
  }
  if (!choice) throw new Error(id ? "Your selected wallet is unavailable. Open it and retry, or select another wallet." : choices.length ? "Select a wallet to continue." : "Your wallet is unavailable. Open your wallet and retry restoring the session.");
  return choice;
}
export function activateWallet(choice: WalletChoice) {
  state().active = choice;
  // Only a public provider identifier is persisted; credentials remain in the HttpOnly cookie.
  try { window.localStorage?.setItem(preferenceKey, choice.id); } catch { /* Login still works when preference storage is blocked. */ }
}
export function getWalletProvider(): Provider {
  const selected = state().active;
  if (selected) return selected.provider;
  const choices = state().choices.size ? [...state().choices.values()] : legacyChoices();
  const id = preferredWalletId();
  const choice = id ? choices.find(value => value.id === id) : choices.length === 1 ? choices[0] : undefined;
  if (!choice) throw new Error("Your selected wallet is unavailable. Open it and retry.");
  return choice.provider;
}
