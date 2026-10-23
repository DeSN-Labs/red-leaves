// Local integration only: starts Anvil and a temporary NAS. Never reads deployment secrets.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { createServer } from "node:net";
import { fileURLToPath } from "node:url";
import { createPublicClient, createWalletClient, http, keccak256, bytesToHex } from "viem";
import { baseSepolia } from "viem/chains";
import { encodeCanonical } from "../src/content.ts";

const root = fileURLToPath(new URL("../../", import.meta.url));
const directory = await mkdtemp(resolve(tmpdir(), "red-leaves-publishing-"));
const children = [];
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
async function port() { const server = createServer(); await new Promise(resolve => server.listen(0,"127.0.0.1",resolve)); const p = server.address().port; await new Promise(resolve => server.close(resolve)); return p; }
function child(command,args,options={}) { const process = spawn(command,args,{stdio:["ignore","pipe","pipe"],...options}); children.push(process); return process; }
async function until(task) { let last; for(let i=0;i<100;i++) { try {return await task();} catch(error) {last=error; await pause(100);} } throw last; }
try {
  const rpc = `http://127.0.0.1:${await port()}`;
  child("anvil",["--silent","--chain-id","84532","--port",rpc.split(":").at(-1)]);
  const publicClient = createPublicClient({chain:baseSepolia,transport:http(rpc)});
  await until(() => publicClient.getBlockNumber({cacheTime:0}));
  const addresses = await publicClient.request({method:"eth_accounts"});
  // Anvil's unlocked development accounts; no private key is loaded from the user's files.
  const wallets = addresses.slice(0,3).map(account => createWalletClient({account,chain:baseSepolia,transport:http(rpc)}));
  async function deploy(name) {
    const artifact = JSON.parse(await readFile(resolve(root,`contracts/out/${name}.sol/${name}.json`),"utf8"));
    const hash = await wallets[0].deployContract({abi:artifact.abi,bytecode:artifact.bytecode.object});
    const receipt = await publicClient.waitForTransactionReceipt({hash});
    return {address:receipt.contractAddress,abi:artifact.abi};
  }
  const registry = await deploy("UserRegistry");
  const api = `http://127.0.0.1:${await port()}`;
  const node = child(resolve(root,"target/debug/red-leaves"),[],{cwd:directory,env:{...process.env,
    RED_LEAVES_LISTEN:api.slice(7),RED_LEAVES_DATABASE_URL:`sqlite://${resolve(directory,"nas.sqlite")}`,
    RED_LEAVES_AUTH_URI:"http://127.0.0.1:5173",RED_LEAVES_NODE_KEY_PATH:resolve(directory,"node.key"),
    RED_LEAVES_RPC_URL:rpc,RED_LEAVES_USER_REGISTRY_ADDRESS:registry.address,
    RED_LEAVES_CONTENT_PATH:resolve(directory,"content"),RED_LEAVES_USER_KEY_PATH:resolve(directory,"user-keys"),RED_LEAVES_MASTER_KEY_PATH:resolve(directory,"master.key"),
  }});
  let nodeLog=""; node.stderr.on("data",data=>{nodeLog+=data.toString();});
  await until(async()=>{if(node.exitCode!==null) throw new Error(`NAS exited: ${nodeLog}`); const response=await fetch(`${api}/api/auth/me`); assert.equal(response.status,401);});
  async function request(path,body,token) {
    const response=await fetch(`${api}/api/${path}`,{method:body===undefined?"GET":"POST",headers:{"Content-Type":"application/json",...(token?{Authorization:`Bearer ${token}`}:{})},...(body===undefined?{}:{body:JSON.stringify(body)})});
    return {status:response.status,body:JSON.parse((await response.text()) || "null")};
  }
  const sessions=[];
  for (const wallet of wallets) {
    const {body:challenge}=await request("auth/challenge",{address:wallet.account.address});
    const signature=await wallet.signMessage({message:challenge.message});
    const {body:session,status}=await request("auth/verify",{challenge_id:challenge.challenge_id,verifier:challenge.verifier,signature});
    assert.equal(status,200);sessions.push(session.token);
    const {body:nas}=await request("nas/info",undefined,session.token);
    const {body:key}=await request("content/key",undefined,session.token);
    const hash=await wallet.writeContract({address:registry.address,abi:registry.abi,functionName:"register",args:[nas.peer_id,key.public_key,"Local test user"]});
    await publicClient.waitForTransactionReceipt({hash});
  }
  async function publish(directed,overrides={}) {
    const action={action:"publish",post:{title:"Local integration post",body:"Only the signed audience may read this content",distribution:directed?"directed":"broadcast",recipients:directed?[Array.from(Buffer.from(wallets[1].account.address.slice(2),"hex"))]:[],...overrides}};
    const {body:prepared,status}=await request("social/prepare",{action},sessions[0]);assert.equal(status,200,JSON.stringify(prepared));
    const intent=prepared.intent;
    const encoded=encodeCanonical({...intent,scope:{...intent.scope,registry_contract:new Uint8Array(intent.scope.registry_contract)},actor:new Uint8Array(intent.actor),nonce:new Uint8Array(intent.nonce),action:{action:"publish",post:{...intent.action.post,recipients:intent.action.post.recipients.map(v=>new Uint8Array(v))}}});
    assert.equal(bytesToHex(encoded),prepared.payload);assert.equal(keccak256(encoded),prepared.payload_hash);
    const signature=await wallets[0].signTypedData({domain:{name:"RedLeaves",version:"1",chainId:84532,verifyingContract:registry.address},types:{SocialAction:[{name:"payloadHash",type:"bytes32"}]},primaryType:"SocialAction",message:{payloadHash:prepared.payload_hash}});
    const committed=await request("social/commit",{payload:prepared.payload,signature},sessions[0]);assert.equal(committed.status,200,JSON.stringify(committed.body));return committed.body;
  }
  const image=Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+j2ZkAAAAASUVORK5CYII=","base64");
  async function upload() {
    const response=await fetch(`${api}/api/media`,{method:"POST",headers:{"Content-Type":"image/png",Authorization:`Bearer ${sessions[0]}`},body:image});
    assert.equal(response.status,200,await response.clone().text());return response.json();
  }
  async function readMedia(id,file,actor) { return fetch(`${api}/api/posts/${id}/media/${file.id}`,{headers:{Authorization:`Bearer ${sessions[actor]}`}}); }
  const attachment=await upload();
  const post = await publish(true,{body:"",media:[attachment]});
  assert.equal(post.status,"saved_pending_network");
  assert.equal(post.price_wei,undefined);
  const content=await request(`posts/${post.id}`,undefined,sessions[1]);
  assert.equal(content.status,200,JSON.stringify(content.body));
  assert.equal(content.body.intent.action.post.media[0].id,attachment.id);
  assert.equal(content.body.locked,undefined);
  assert.equal(content.body.intent.action.post.price_wei,undefined);
  assert.equal((await request(`posts/${post.id}`,undefined,sessions[2])).status,404);
  assert.equal((await request(`posts/${post.id}/unlock`,{},sessions[1])).status,404);
  assert.equal((await request("media",undefined,sessions[1])).body.media.length,0);
  const mediaResponse=await readMedia(post.id,attachment,1);assert.equal(mediaResponse.status,200);assert.deepEqual(Buffer.from(await mediaResponse.arrayBuffer()),image);
  assert.equal((await readMedia(post.id,attachment,2)).status,404);
  const publicAttachment=await upload();
  const photoPost=await publish(false,{title:"Public photo",body:"",media:[publicAttachment]});
  assert.deepEqual(Buffer.from(await (await readMedia(photoPost.id,publicAttachment,2)).arrayBuffer()),image);
  const broadcast=await publish(false);assert.equal((await request(`posts/${broadcast.id}`,undefined,sessions[2])).status,200);
  console.log("Local Anvil/NAS integration passed: registry keys, client CBOR/signature, immediate publication, audience isolation, private uploads, signed media, public broadcasts and removed payment routes.");
} finally {
  for(const process of children.reverse()) process.kill("SIGTERM");
  await pause(200);await rm(directory,{recursive:true,force:true});
}
