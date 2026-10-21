import test from "node:test";
import assert from "node:assert/strict";
import { getNasInfo, checkNasSession, connectWallet, logoutWallet, sessionExpiresAt, nasApi, nasBlob, nasUpload, watchNasChanges } from "../src/wallet.ts";

const address = `0x${"12".repeat(20)}`;
const challenge = { challenge_id: "id", verifier: "secret", message: "Sign in to Red Leaves" };
function provider(overrides = {}) {
  globalThis.window = { ethereum: { async request({ method, params }) {
    if (method in overrides) return overrides[method](params);
    if (method === "eth_requestAccounts" || method === "eth_accounts") return [address];
    if (method === "eth_chainId") return "0x14a34";
    if (method === "personal_sign") {
      assert.equal(Buffer.from(params[0].slice(2), "hex").toString(), challenge.message);
      assert.equal(params[1], address);
      return "0xsignature";
    }
    throw new Error(`Unexpected method ${method}`);
  } } };
}
function response(body, status = 200) { return new Response(status === 204 ? null : JSON.stringify(body), { status }); }

test("Web signs a challenge and revokes the Bearer session without cookies", async () => {
  provider(); const calls = [];
  globalThis.fetch = async (url, options) => {
    calls.push([url, options]); assert.equal(options.credentials, "omit");
    assert.equal(new URL(url).origin, "http://127.0.0.1:3000");
    if (url.endsWith("challenge")) return response(challenge);
    if (url.endsWith("verify")) {
      assert.deepEqual(JSON.parse(options.body), { challenge_id: "id", verifier: "secret", signature: "0xsignature" });
      return response({ token: "token", address, expires_at: 100 });
    }
    assert.equal(options.headers.Authorization, "Bearer token");
    if (url.endsWith("info")) return response({ node_type: "nas", peer_id: "12D3KooWTestPeerId", p2p_running: false });
    if (url.endsWith("me")) {
      assert.equal(options.method, "GET");
      return response({ address, chain_id: 84532, expires_at: 100 });
    }
    return response(null, 204);
  };
  assert.equal(await connectWallet(), address); assert.equal(sessionExpiresAt(), 100);
  assert.equal((await checkNasSession()).address, address);
  assert.deepEqual(await getNasInfo(), { node_type: "nas", peer_id: "12D3KooWTestPeerId", p2p_running: false });
  await logoutWallet(); assert.equal(sessionExpiresAt(), 0); assert.equal(calls.length, 5);
  await assert.rejects(checkNasSession(), /Sign in with your wallet/);
});

test("wrong chain and rejected signature never establish a session", async () => {
  provider({ eth_chainId: () => "0x1" });
  globalThis.fetch = async () => { throw new Error("API should not be called"); };
  await assert.rejects(connectWallet(), /Base Sepolia/);
  provider({ personal_sign: () => { throw Object.assign(new Error("Rejected"), { code: 4001 }); } });
  globalThis.fetch = async url => { assert.ok(url.endsWith("challenge")); return response(challenge); };
  await assert.rejects(connectWallet(), { code: 4001 }); assert.equal(sessionExpiresAt(), 0);
});

test("account change during signing aborts before verification", async () => {
  provider({ eth_accounts: () => [`0x${"34".repeat(20)}`] });
  globalThis.fetch = async url => { assert.ok(url.endsWith("challenge")); return response(challenge); };
  await assert.rejects(connectWallet(), /Wallet state changed/);
});

test("logout racing verification revokes the newly issued session", async () => {
  provider(); let release, started;
  const pending = new Promise(resolve => { started = resolve; });
  const issued = new Promise(resolve => { release = resolve; }); let revoked = false;
  globalThis.fetch = async (url, options) => {
    if (url.endsWith("challenge")) return response(challenge);
    if (url.endsWith("verify")) { started(); return issued; }
    assert.equal(options.headers.Authorization, "Bearer late-token"); revoked = true;
    return response(null, 204);
  };
  const login = connectWallet();
  await pending; await logoutWallet();
  release(response({ token: "late-token", address, expires_at: 100 }));
  await assert.rejects(login, /Wallet state changed/); assert.ok(revoked); assert.equal(sessionExpiresAt(), 0);
});

test("NAS checks reject unavailable or revoked server sessions", async () => {
  provider();
  globalThis.fetch = async url => {
    if (url.endsWith("challenge")) return response(challenge);
    if (url.endsWith("verify")) return response({ token: "token", address, expires_at: 100 });
    if (url.endsWith("me")) return response({ error: "Invalid or expired authentication" }, 401);
    return response(null, 204);
  };
  await connectWallet();
  await assert.rejects(checkNasSession(), /NAS authentication failed \(401\)/);
  globalThis.fetch = async url => {
    if (url.endsWith("me")) throw new TypeError("Failed to fetch");
    return response(null, 204);
  };
  await assert.rejects(checkNasSession(), /Failed to fetch/);
  await logoutWallet();
});

test("profile registration requires a NAS node and explains outdated server endpoints", async () => {
  provider();
  globalThis.fetch=async url=>{
    if(url.endsWith("challenge"))return response(challenge);
    if(url.endsWith("verify"))return response({token:"token",address,expires_at:100});
    return response(null,204);
  };
  await connectWallet();
  globalThis.fetch=async()=>response({node_type:"relay",peer_id:"peer",p2p_running:false});
  await assert.rejects(getNasInfo(),/requires a NAS node/);
  globalThis.fetch=async()=>response({node_type:"nas",peer_id:"",p2p_running:false});
  await assert.rejects(getNasInfo(),/invalid PeerId/);
  globalThis.fetch=async()=>response({error:"Not found"},404);
  await assert.rejects(getNasInfo(),/Restart the NAS server/);
  globalThis.fetch=async()=>response(null,204);
  await logoutWallet();
});

test("content requests use Bearer authentication and reject responses after logout", async () => {
  provider(); let release, started;
  const pending = new Promise(resolve => { started = resolve; });
  const result = new Promise(resolve => { release = resolve; });
  globalThis.fetch = async (url, options) => {
    if (url.endsWith("challenge")) return response(challenge);
    if (url.endsWith("verify")) return response({ token: "content-token", address, expires_at: 100 });
    assert.equal(options.credentials, "omit");
    assert.equal(options.headers.Authorization, "Bearer content-token");
    if (url.endsWith("logout")) return response(null, 204);
    assert.ok(url.endsWith("/api/posts")); return { ok: true, async json() { started(); return result; } };
  };
  await connectWallet(); const request = nasApi("posts"); await pending;
  await logoutWallet(); release({ posts: [] });
  await assert.rejects(request, /Wallet state changed/);
  await assert.rejects(nasApi("posts"), /Sign in/);
});

test("media transfers use the session and discard blob responses after logout", async () => {
 provider(); let release, started;
 const pending = new Promise(resolve => { started = resolve; });
 const result = new Promise(resolve => { release = resolve; });
 globalThis.fetch = async (url, options) => {
   if (url.endsWith("challenge")) return response(challenge);
   if (url.endsWith("verify")) return response({ token: "media-token", address, expires_at: 100 });
   assert.equal(options.headers.Authorization, "Bearer media-token");
   assert.equal(options.credentials, "omit");
   if (url.endsWith("logout")) return response(null, 204);
   if (options.method === "POST") { assert.equal(options.headers["Content-Type"], "image/png"); assert(options.body instanceof File); return response({id:"upload"}); }
   return { ok: true, async blob() { started(); return result; } };
 };
 await connectWallet();
 assert.deepEqual(await nasUpload(new File(["png"], "a.png", {type:"image/png"})), {id:"upload"});
 const request = nasBlob("media/test");await pending;await logoutWallet();release(new Blob(["private"]));
 await assert.rejects(request,/Wallet state changed/);
 await assert.rejects(nasBlob("media/test"),/Sign in/);
});

test("media downloads stay within two concurrent transfers", async () => {
  provider(); let active = 0, maximum = 0, entered = 0;
  const releases = [];
  globalThis.fetch = async url => {
    if (url.endsWith("challenge")) return response(challenge);
    if (url.endsWith("verify")) return response({ token: "media-limit", address, expires_at: 100 });
    if (url.endsWith("logout")) return response(null, 204);
    active++; entered++; maximum = Math.max(maximum, active);
    return { ok: true, async blob() { await new Promise(resolve => releases.push(resolve)); active--; return new Blob(["image"]); } };
  };
  await connectWallet();
  const requests = Array.from({length:5}, (_,i) => nasBlob(`media/${i}`));
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(entered,2);
  for(let i=0; i<5; i++) {
    assert(releases.length > 0); releases.shift()();
    await new Promise(resolve => setTimeout(resolve, 0));
  }
  assert.equal((await Promise.all(requests)).length,5);
  assert.equal(maximum,2);
  await logoutWallet();
});

test("live updates use Bearer SSE, decode fragmented frames and stop on cancellation", async () => {
 provider(); const controller=new AbortController();let updates=0;
 globalThis.fetch=async(url,options)=>{
  if(url.endsWith("challenge"))return response(challenge);
  if(url.endsWith("verify"))return response({token:"live",address,expires_at:100});
  if(url.endsWith("logout"))return response(null,204);
  assert(url.endsWith("/api/social/events"));assert(!url.includes("token="));
  assert.equal(options.headers.Authorization,"Bearer live");assert.equal(options.credentials,"omit");
  const encoder=new TextEncoder();
  return new Response(new ReadableStream({start(stream){stream.enqueue(encoder.encode(": keepalive\n\nevent: sy"));stream.enqueue(encoder.encode("nc\ndata: {}\n\nevent: update\ndata: {}\n\n"));}}),{headers:{"Content-Type":"text/event-stream"}});
 };
 await connectWallet();
 await watchNasChanges(()=>{updates++;if(updates===2)controller.abort();},controller.signal);
 assert.equal(updates,2);await logoutWallet();
});
test("live updates never invoke callbacks from a buffered stream after logout", async () => {
 provider();const controller=new AbortController();let updates=0,logout;
 globalThis.fetch=async url=>{
  if(url.endsWith("challenge"))return response(challenge);
  if(url.endsWith("verify"))return response({token:"live-race",address,expires_at:100});
  if(url.endsWith("logout"))return response(null,204);
  return new Response(new ReadableStream({start(stream){stream.enqueue(new TextEncoder().encode("event: update\ndata: {}\n\nevent: update\ndata: {}\n\n"));}}));
 };
 await connectWallet();
 await assert.rejects(watchNasChanges(()=>{updates++;logout=logoutWallet();},controller.signal),/Wallet state changed/);
 await logout;assert.equal(updates,1);controller.abort();
});
