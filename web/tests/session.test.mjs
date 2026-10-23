import test from "node:test";
import assert from "node:assert/strict";

const address = `0x${"12".repeat(20)}`;
const metadata = () => ({ address, chain_id: 84532, expires_at: Math.floor(Date.now() / 1000) + 3600 });
let counter = 0;
const freshWallet = () => import(`../src/wallet.ts?refresh=${++counter}`);
const response = (body, status = 200) => new Response(status === 204 ? null : JSON.stringify(body), { status });
function wallet(accounts = [address], chain = "0x14a34") {
  const methods = [];
  globalThis.window = {
    get sessionStorage() { throw new Error("Session storage must not contain credentials"); },
    get localStorage() { throw new Error("Local storage must not contain credentials"); },
    ethereum: { async request({ method }) {
      methods.push(method);
      if (method === "eth_accounts") return accounts;
      if (method === "eth_chainId") return chain;
      throw new Error(`Unexpected wallet prompt: ${method}`);
    } },
  };
  return methods;
}
function cookieFetch(handler) {
  globalThis.fetch = async (url, options) => {
    assert.equal(options.credentials, "include");
    assert.equal(options.headers.Authorization, undefined);
    assert.equal(options.headers["X-Red-Leaves-Client"], "web");
    return handler(url, options);
  };
}

test("fresh page restores cookie session silently without browser storage", async () => {
  const methods = wallet();
  cookieFetch(url => { assert.ok(url.endsWith("/me")); return response(metadata()); });
  const beforeRefresh = await freshWallet();
  assert.equal(await beforeRefresh.restoreWallet(), address);
  const afterRefresh = await freshWallet();
  assert.equal(await afterRefresh.restoreWallet(), address);
  assert.ok(afterRefresh.sessionExpiresAt() > Date.now() / 1000);
  assert.deepEqual(methods, ["eth_accounts", "eth_chainId", "eth_accounts", "eth_chainId"]);
});

test("missing or expired cookie shows login without prompting the wallet", async () => {
  const methods = wallet();
  cookieFetch(() => response({ error: "Expired" }, 401));
  const module = await freshWallet();
  assert.equal(await module.restoreWallet(), null);
  assert.equal(module.sessionExpiresAt(), 0);
  assert.deepEqual(methods, []);
});

test("temporarily unavailable NAS allows retrying the same cookie", async () => {
  wallet(); let offline = true;
  cookieFetch(() => response(offline ? {} : metadata(), offline ? 503 : 200));
  const module = await freshWallet();
  await assert.rejects(module.restoreWallet(), /503/);
  offline = false;
  assert.equal(await module.restoreWallet(), address);
});

test("wallet temporarily unavailable preserves cookie for a later retry", async () => {
  wallet(); delete window.ethereum;
  cookieFetch(url => { assert.ok(url.endsWith("/me")); return response(metadata()); });
  const module = await freshWallet();
  await assert.rejects(module.restoreWallet(), /wallet is unavailable/);
  wallet();
  assert.equal(await module.restoreWallet(), address);
});

for (const [name, accounts, chain] of [
  ["account changed", [`0x${"34".repeat(20)}`], "0x14a34"],
  ["wrong chain", [address], "0x1"],
  ["wallet locked", [], "0x14a34"],
]) {
  test(`${name} revokes the cookie rather than restoring another identity`, async () => {
    wallet(accounts, chain); let revoked = false;
    cookieFetch(url => {
      if (url.endsWith("/logout")) { revoked = true; return response(null, 204); }
      return response(metadata());
    });
    const module = await freshWallet();
    assert.equal(await module.restoreWallet(), null);
    assert.ok(revoked);
    assert.equal(module.sessionExpiresAt(), 0);
  });
}

test("invalid NAS session metadata is discarded", async () => {
  wallet(); let revoked = false;
  cookieFetch(url => {
    if (url.endsWith("/logout")) { revoked = true; return response(null, 204); }
    return response({ ...metadata(), expires_at: 0 });
  });
  const module = await freshWallet();
  assert.equal(await module.restoreWallet(), null);
  assert.ok(revoked);
});

test("logout during restoration cannot reactivate the old session", async () => {
  wallet(); let release;
  const pending = new Promise(resolve => { release = resolve; });
  cookieFetch(url => url.endsWith("/me") ? pending : response(null, 204));
  const module = await freshWallet();
  const restoring = module.restoreWallet();
  await module.logoutWallet();
  release(response(metadata()));
  await assert.rejects(restoring, /Wallet state changed/);
  assert.equal(module.sessionExpiresAt(), 0);
});

test("concurrent restoration calls share one request", async () => {
  wallet(); let requests = 0;
  cookieFetch(() => { requests++; return response(metadata()); });
  const module = await freshWallet();
  const first = module.restoreWallet(), second = module.restoreWallet();
  assert.equal(first, second);
  assert.deepEqual(await Promise.all([first, second]), [address, address]);
  assert.equal(requests, 1);
});

test("logout removes local access even when NAS revocation fails", async () => {
  wallet();
  cookieFetch(() => response(metadata()));
  const module = await freshWallet();
  await module.restoreWallet();
  cookieFetch(() => response({}, 503));
  await assert.rejects(module.logoutWallet(), /503/);
  assert.equal(module.sessionExpiresAt(), 0);
  await assert.rejects(module.nasApi("posts"), /Sign in/);
});
