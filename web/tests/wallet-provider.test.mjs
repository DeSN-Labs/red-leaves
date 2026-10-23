import test from "node:test";
import assert from "node:assert/strict";
import { availableWallets, chooseWallet, activateWallet, getWalletProvider, preferredWalletId } from "../src/wallet-provider.ts";
const address = `0x${"12".repeat(20)}`;
const metadata = () => ({ address, chain_id: 84532, expires_at: Math.floor(Date.now() / 1000) + 3600 });
let counter = 0;
const freshWallet = () => import(`../src/wallet.ts?providers=${++counter}`);
function storage() {
  const values = new Map();
  return { values, getItem: key => values.get(key) ?? null, setItem: (key, value) => values.set(key, value) };
}
function provider(reject = false) {
  const methods = [];
  return { methods, async request({ method }) {
    methods.push(method);
    if (method === "eth_requestAccounts" || method === "eth_accounts") return [address];
    if (method === "eth_chainId") return "0x14a34";
    if (method === "personal_sign") { if (reject) throw new Error("Signature rejected"); return "0xsignature"; }
    throw new Error(`Unexpected method ${method}`);
  } };
}
function page(wallets, saved, legacy) {
  const target = new EventTarget();
  target.localStorage = saved;
  target.ethereum = legacy ?? { request() { throw new Error("The generic injected wallet must not be called"); } };
  target.addEventListener("eip6963:requestProvider", () => {
    for (const [rdns, wallet] of wallets) target.dispatchEvent(new CustomEvent("eip6963:announceProvider", { detail: { info: { rdns, name: rdns, uuid: String(++counter) }, provider: wallet } }));
  });
  globalThis.window = target;
  return target;
}
function api() {
  globalThis.fetch = async (url, options) => {
    assert.equal(options.credentials, "include");
    const body = url.endsWith("challenge") ? { challenge_id: "id", verifier: "secret", message: "Sign in" } : metadata();
    return new Response(url.endsWith("logout") ? null : JSON.stringify(body), { status: url.endsWith("logout") ? 204 : 200 });
  };
}

test("remember selected wallet across refresh even when provider order and UUID change", async () => {
  const saved = storage(), first = provider(), selected = provider();
  page([["io.first", first], ["io.selected", selected]], saved); api();
  const choices = await availableWallets();
  assert.equal(choices.length, 2);
  const original = await freshWallet();
  assert.equal(await original.connectWallet(choices.find(choice => choice.id === "eip6963:io.selected")), address);
  assert.equal(preferredWalletId(), "eip6963:io.selected");
  assert.deepEqual([...saved.values], [["red-leaves:wallet-provider", "eip6963:io.selected"]]);
  const nextSelected = provider(), other = provider();
  page([["io.selected", nextSelected], ["io.first", other]], saved);
  const refreshed = await freshWallet();
  assert.equal(await refreshed.restoreWallet(), address);
  assert.deepEqual(nextSelected.methods, ["eth_accounts", "eth_chainId"]);
  assert.deepEqual(other.methods, []);
  assert.deepEqual(first.methods, []);
  assert.equal(getWalletProvider(), nextSelected);
});

test("multiple wallets require an explicit first choice and never invoke the generic provider", async () => {
  const a = provider(), b = provider();
  page([["io.a", a], ["io.b", b]], storage());
  await assert.rejects(chooseWallet(), /Select a wallet/);
  assert.deepEqual(a.methods, []); assert.deepEqual(b.methods, []);
});

test("missing remembered extension preserves cookie and never silently switches wallet", async () => {
  const saved = storage(); saved.setItem("red-leaves:wallet-provider", "eip6963:io.missing");
  const other = provider(); page([["io.other", other]], saved);
  const calls = []; api(); const fetch = globalThis.fetch;
  globalThis.fetch = (url, options) => { calls.push(url); return fetch(url, options); };
  const module = await freshWallet();
  await assert.rejects(module.restoreWallet(), /selected wallet is unavailable/);
  assert(calls.every(url => url.endsWith("/me")));
  assert.equal(preferredWalletId(), "eip6963:io.missing");
  assert.deepEqual(other.methods, []);
});

test("remembered extension may announce after initial discovery", async () => {
  const saved = storage(); saved.setItem("red-leaves:wallet-provider", "eip6963:io.late");
  const selected = provider(), target = page([], saved);
  const timer = setTimeout(() => target.dispatchEvent(new CustomEvent("eip6963:announceProvider", { detail: { info: { rdns: "io.late", name: "Late wallet" }, provider: selected } })), 400);
  try { assert.equal((await chooseWallet()).provider, selected); } finally { clearTimeout(timer); }
});

test("only successful explicit login replaces the remembered wallet", async () => {
  const saved = storage(), a = provider(), rejected = provider(true), b = provider();
  page([["io.a", a], ["io.rejected", rejected], ["io.b", b]], saved); api();
  const choices = await availableWallets(), module = await freshWallet();
  await module.connectWallet(choices[0]);
  await assert.rejects(module.connectWallet(choices[1]), /Signature rejected/);
  assert.equal(preferredWalletId(), choices[0].id);
  await module.connectWallet(choices[2]);
  assert.equal(preferredWalletId(), choices[2].id);
  await module.logoutWallet();
  assert.equal(preferredWalletId(), choices[2].id);
});

test("legacy provider arrays retain brand selection after reordering", async () => {
  const saved = storage(), a = Object.assign(provider(), { isMetaMask: true }), b = Object.assign(provider(), { isCoinbaseWallet: true });
  page([], saved, { providers: [a, b] });
  activateWallet((await availableWallets()).find(choice => choice.name === "Coinbase Wallet"));
  page([], saved, { providers: [b, a] });
  assert.equal((await chooseWallet()).provider, b);
});

test("repeated or conflicting announcements cannot replace an active provider", async () => {
  const original = provider(), replacement = provider(), target = page([["io.wallet", original]], storage());
  activateWallet((await availableWallets())[0]);
  target.dispatchEvent(new CustomEvent("eip6963:announceProvider", { detail: { info: { rdns: "io.wallet", name: "Replacement" }, provider: replacement } }));
  assert.equal(getWalletProvider(), original);
  assert.equal((await chooseWallet()).provider, original);
});

test("remembered wallet account mismatch does not fall back to another matching wallet", async () => {
  const saved = storage(); saved.setItem("red-leaves:wallet-provider", "eip6963:io.selected");
  const selected = provider(), other = provider(), request = selected.request.bind(selected);
  selected.request = args => args.method === "eth_accounts" ? Promise.resolve([`0x${"34".repeat(20)}`]) : request(args);
  page([["io.selected", selected], ["io.other", other]], saved); api();
  const module = await freshWallet();
  assert.equal(await module.restoreWallet(), null);
  assert.deepEqual(other.methods, []);
  assert.equal(module.sessionExpiresAt(), 0);
});

test("account change during verification revokes cookie and does not remember the login", async () => {
  const saved = storage(), selected = provider(), request = selected.request.bind(selected);
  let changed = false, revoked = false;
  selected.request = args => changed && args.method === "eth_accounts" ? Promise.resolve([`0x${"34".repeat(20)}`]) : request(args);
  page([["io.wallet", selected]], saved); api(); const fetch = globalThis.fetch;
  globalThis.fetch = (url, options) => { if (url.endsWith("verify")) changed = true; if (url.endsWith("logout")) revoked = true; return fetch(url, options); };
  const module = await freshWallet();
  await assert.rejects(module.connectWallet(), /Wallet state changed/);
  assert.equal(preferredWalletId(), null);
  assert.equal(module.sessionExpiresAt(), 0);
  assert.ok(revoked);
});
