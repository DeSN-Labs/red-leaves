import test from "node:test";
import assert from "node:assert/strict";
import { decodeFunctionData, encodeFunctionResult } from "viem";
import { createRegistry, registryAbi, validateRegistration } from "../src/registry.ts";

const owner = `0x${"11".repeat(20)}`;
const contract = `0x${"22".repeat(20)}`;
const input = { nasPeerId: "12D3KooWTestIdentity", encryptionPublicKey: "0x010203", nickname: "Alice" };
function storage() {
  const values = new Map();
  return { getItem: key => values.get(key) ?? null, setItem: (key,value) => values.set(key,value), removeItem: key => values.delete(key) };
}
function mock({ registered = false, failRead = false, chain = "0x14a34", code = "0x6000", reject = false } = {}) {
  const calls = [];
  const provider = { async request({method,params}) {
    calls.push(method);
    if (method === "eth_chainId") return chain;
    if (method === "eth_accounts") return [owner];
    if (method === "eth_getCode") return code;
    if (method === "eth_blockNumber") return "0x10";
    if (method === "eth_call") {
      if (failRead) throw new Error("RPC unavailable");
      const {functionName} = decodeFunctionData({abi: registryAbi,data:params[0].data});
      if (functionName === "isRegistered") return encodeFunctionResult({abi:registryAbi,functionName,result:registered});
      if (functionName === "getUser") return encodeFunctionResult({abi:registryAbi,functionName,result:input});
      return "0x";
    }
    if (method === "eth_sendTransaction") {
      if (reject) throw Object.assign(new Error("User rejected the request"), {code:4001});
      throw new Error("Unexpected transaction broadcast");
    }
    throw new Error(`Unexpected RPC method ${method}`);
  }};
  return { registry:createRegistry(provider,contract,owner,storage()), calls };
}

test("registration validation uses byte lengths and rejects incomplete input", () => {
  validateRegistration(input);
  for (const invalid of [ { ...input,nasPeerId:"" }, { ...input,nasPeerId:"p".repeat(129) }, { ...input,encryptionPublicKey:"0x0" }, { ...input,encryptionPublicKey:"0x"+"01".repeat(257) }, { ...input,nickname:" " }, { ...input,nickname:"\u00e9".repeat(65) } ]) {
    assert.throws(() => validateRegistration(invalid));
  }
});
test("registry configuration rejects missing and zero addresses", () => {
  for (const address of ["", "invalid", `0x${"00".repeat(20)}`]) assert.throws(() => createRegistry({},address,owner,storage()),/VITE_USER_REGISTRY_ADDRESS/);
});
test("login-time reads never submit transactions and distinguish existing registration", async () => {
  const empty=mock(); assert.equal(await empty.registry.read(),null);
  const existing=mock({registered:true}); assert.deepEqual(await existing.registry.read(),input);
  assert.ok(!empty.calls.includes("eth_sendTransaction") && !existing.calls.includes("eth_sendTransaction"));
});
test("wrong network, missing deployment and RPC failure are not treated as unregistered", async () => {
  await assert.rejects(mock({chain:"0x1"}).registry.read(),/Base Sepolia/);
  await assert.rejects(mock({code:"0x"}).registry.read(),/No registry contract/);
  await assert.rejects(mock({failRead:true}).registry.read());
});
test("already registered wallets and canceled sessions cannot send registration", async () => {
  const existing=mock({registered:true});
  await assert.rejects(existing.registry.register(input,()=>{}),/already registered/);
  const canceled=mock();
  await assert.rejects(canceled.registry.register(input,()=>{},()=>false),/session changed/);
  assert.ok(!existing.calls.includes("eth_sendTransaction") && !canceled.calls.includes("eth_sendTransaction"));
});
test("wallet rejection preserves login and does not create a pending transaction", async () => {
  const {registry,calls}=mock({reject:true});
  await assert.rejects(registry.register(input,()=>{}));
  assert.ok(calls.includes("eth_sendTransaction"));
  assert.equal(registry.pending(),null);
});
test("pending transactions are scoped to registry and wallet and prevent duplicate writes", async () => {
  const store=storage();
  const key=`red-leaves:pending-registration:84532:${contract}:${owner}`;
  const value={...input,hash:`0x${"33".repeat(32)}`};
  store.setItem(key,JSON.stringify(value));
  const provider={async request(){throw new Error("Unexpected RPC request");}};
  const registry=createRegistry(provider,contract,owner,store);
  assert.deepEqual(registry.pending(),value);
  await assert.rejects(registry.register(input,()=>{}),/Resume the pending/);
  assert.equal(createRegistry(provider,`0x${"44".repeat(20)}`,owner,store).pending(),null);
  assert.equal(createRegistry(provider,contract,`0x${"55".repeat(20)}`,store).pending(),null);
});
