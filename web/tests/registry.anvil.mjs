// Explicit local integration test: node --experimental-strip-types --test tests/registry.anvil.mjs
import test from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createServer } from "node:net";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { encodeFunctionData } from "viem";
import { createRegistry, registryAbi } from "../src/registry.ts";

function storage() {
  const values=new Map();
  return {getItem:key=>values.get(key)??null,setItem:(key,value)=>values.set(key,value),removeItem:key=>values.delete(key)};
}

test("local registry submission, two-block confirmation, resume and revert",{timeout:45000},async t=>{
  const server=createServer();
  await new Promise(resolve=>server.listen(0,"127.0.0.1",resolve));
  const port=server.address().port;await new Promise(resolve=>server.close(resolve));
  const node=spawn("anvil",["--silent","--chain-id","84532","--port",String(port),"--block-time","1"],{stdio:"ignore"});
  t.after(()=>node.kill("SIGTERM"));
  const rpc=async(method,params=[])=>{
    const response=await fetch(`http://127.0.0.1:${port}`,{method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify({jsonrpc:"2.0",id:1,method,params}),signal:AbortSignal.timeout(3000)});
    const body=await response.json();
    if(body.error)throw Object.assign(new Error(body.error.message),{code:body.error.code});
    return body.result;
  };
  for(let attempt=0;;attempt++){
    try{await rpc("eth_chainId");break;}catch(error){if(attempt>=50)throw error;await new Promise(resolve=>setTimeout(resolve,100));}
  }
  const accounts=await rpc("eth_accounts");
  const artifact=JSON.parse(await readFile(fileURLToPath(new URL("../../contracts/out/UserRegistry.sol/UserRegistry.json",import.meta.url)),"utf8"));
  const deployment=await rpc("eth_sendTransaction",[{from:accounts[0],data:artifact.bytecode.object,gas:"0x500000"}]);
  async function receipt(hash){
    for(let attempt=0;attempt<100;attempt++){
      const value=await rpc("eth_getTransactionReceipt",[hash]);if(value)return value;
      await new Promise(resolve=>setTimeout(resolve,100));
    }
    throw new Error("Local transaction was not mined");
  }
  const contract=(await receipt(deployment)).contractAddress;
  const provider={request:({method,params})=>rpc(method,params)};
  const input={nasPeerId:"local-test-peer",encryptionPublicKey:"0x1234",nickname:"Alice"};
  const store=storage();
  const registry=createRegistry(provider,contract,accounts[0],store);
  assert.equal(await registry.read(),null);
  let submitted;
  assert.deepEqual(await registry.register(input,hash=>{submitted=hash;}),input);
  assert.match(submitted,/^0x[0-9a-f]{64}$/);
  assert.deepEqual(await registry.read(),input);assert.equal(registry.pending(),null);
  await assert.rejects(registry.register(input,()=>{}),/already registered/);

  const second={...input,nickname:"Bob"};
  const pendingHash=await rpc("eth_sendTransaction",[{from:accounts[1],to:contract,data:encodeFunctionData({abi:registryAbi,functionName:"register",args:[second.nasPeerId,second.encryptionPublicKey,second.nickname]}),gas:"0x100000"}]);
  const pendingKey=`red-leaves:pending-registration:84532:${contract.toLowerCase()}:${accounts[1].toLowerCase()}`;
  store.setItem(pendingKey,JSON.stringify({...second,hash:pendingHash}));
  const reloaded=createRegistry(provider,contract,accounts[1],store);
  assert.deepEqual(await reloaded.resume(()=>{}),second);assert.equal(reloaded.pending(),null);

  const reverted=await rpc("eth_sendTransaction",[{from:accounts[2],to:contract,data:encodeFunctionData({abi:registryAbi,functionName:"register",args:[input.nasPeerId,"0x",input.nickname]}),gas:"0x100000"}]);
  store.setItem(`red-leaves:pending-registration:84532:${contract.toLowerCase()}:${accounts[2].toLowerCase()}`,JSON.stringify({...input,hash:reverted}));
  const failed=createRegistry(provider,contract,accounts[2],store);
  await assert.rejects(failed.resume(()=>{}),/reverted or was canceled/);
  assert.equal(failed.pending(),null);assert.equal(await failed.read(),null);
});
