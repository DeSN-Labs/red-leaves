// Real libp2p circuits: three temporary NAS processes share one temporary public relay.
// Uses only Anvil's unlocked development accounts, never user deployment credentials.
import assert from "node:assert/strict";
import { lanNodes } from "./lan-nodes.mjs";
import { spawn } from "node:child_process";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { createServer } from "node:net";
import { fileURLToPath } from "node:url";
import { createPublicClient, createWalletClient, http, keccak256, bytesToHex } from "viem";
import { baseSepolia } from "viem/chains";
import { encodeCanonical, actionsMatch } from "../src/content.ts";
const root=fileURLToPath(new URL("../../",import.meta.url));
const directory=await mkdtemp(resolve(tmpdir(),"red-leaves-delivery-"));
const children=[];
let lan=null;
const pause=ms=>new Promise(resolve=>setTimeout(resolve,ms));
async function port(){const server=createServer();await new Promise(resolve=>server.listen(0,"127.0.0.1",resolve));const p=server.address().port;await new Promise(resolve=>server.close(resolve));return p;}
function child(command,args,options={}){const process=spawn(command,args,{stdio:["ignore","pipe","pipe"],...options});process.log="";process.stdout.on("data",d=>process.log+=d.toString());process.stderr.on("data",d=>process.log+=d.toString());children.push(process);return process;}
async function until(task,label="condition",seconds=45){let last;for(let i=0;i<seconds*5;i++){try{return await task();}catch(error){last=error;await pause(200);}}throw new Error(`${label}: ${last?.message}`);}
async function stop(process){if(process.exitCode!==null)return;if(process.stop)await process.stop();else process.kill("SIGTERM");if(process.exitCode===null)await new Promise(resolve=>process.once("exit",resolve));}
try {
 const rpc=`http://127.0.0.1:${await port()}`;
 child("anvil",["--silent","--chain-id","84532","--port",rpc.split(":").at(-1)]);
 const client=createPublicClient({chain:baseSepolia,transport:http(rpc)});
 await until(()=>client.getBlockNumber({cacheTime:0}));
 const addresses=await client.request({method:"eth_accounts"});
 const wallets=addresses.slice(0,4).map(account=>createWalletClient({account,chain:baseSepolia,transport:http(rpc)}));
 async function deploy(name){const artifact=JSON.parse(await readFile(resolve(root,`contracts/out/${name}.sol/${name}.json`),"utf8"));const hash=await wallets[0].deployContract({abi:artifact.abi,bytecode:artifact.bytecode.object});return {address:(await client.waitForTransactionReceipt({hash})).contractAddress,abi:artifact.abi};}
 const registry=await deploy("UserRegistry");
 lan=await lanNodes(process.argv.find(arg=>arg.startsWith("--lan="))?.slice(6),child,port,rpc);
 async function startNode(name,relay){
  if(lan)return lan.start(name,relay,registry.address);
  const path=resolve(directory,name);await import("node:fs/promises").then(fs=>fs.mkdir(path,{recursive:true}));
  const api=`http://127.0.0.1:${await port()}`;
  const env={...process.env,RED_LEAVES_LISTEN:api.slice(7),RED_LEAVES_DATABASE_URL:`sqlite://${resolve(path,"nas.sqlite")}`,RED_LEAVES_AUTH_URI:"http://127.0.0.1:5173",RED_LEAVES_NODE_KEY_PATH:resolve(path,"node.key"),RED_LEAVES_RPC_URL:rpc,RED_LEAVES_USER_REGISTRY_ADDRESS:registry.address,RED_LEAVES_CONTENT_PATH:resolve(path,"content"),RED_LEAVES_USER_KEY_PATH:resolve(path,"keys"),RED_LEAVES_MASTER_KEY_PATH:resolve(path,"master.key"),RED_LEAVES_P2P_ENABLED:"true",RED_LEAVES_P2P_LISTEN:"/ip4/127.0.0.1/tcp/0",RED_LEAVES_P2P_RELAYS:relay??"",RED_LEAVES_P2P_PEERS:"",RED_LEAVES_P2P_RELAY_SERVER:relay?"false":"true"};
  const node={api,path,env};
  node.restart=async()=>{node.process=child(resolve(root,"target/debug/red-leaves"),[],{cwd:path,env});await until(async()=>{assert.equal(node.process.exitCode,null,node.process.log);const response=await fetch(`${api}/api/auth/me`);assert.equal(response.status,401);},`start ${name}`);};
  await node.restart();return node;
 }
 async function request(node,path,body,token=node.token){const response=await fetch(`${node.api}/api/${path}`,{method:body===undefined?"GET":"POST",headers:{"Content-Type":"application/json",...(token?{Authorization:`Bearer ${token}`}:{})},...(body===undefined?{}:{body:JSON.stringify(body)})});return {status:response.status,body:await response.json()};}
 async function login(node,index){const wallet=wallets[index];const challenge=(await request(node,"auth/challenge",{address:wallet.account.address})).body;const signature=await wallet.signMessage({message:challenge.message});const result=await request(node,"auth/verify",{challenge_id:challenge.challenge_id,verifier:challenge.verifier,signature});assert.equal(result.status,200,JSON.stringify(result.body));node.token=result.body.token;node.index=index;node.owner=wallet.account.address.toLowerCase();}
 async function info(node){const response=await request(node,"nas/info");assert.equal(response.status,200);return response.body;}
 let relayAddress;
 if(!lan?.direct){
  const relay=await startNode("relay");await login(relay,3);
  relayAddress=await until(async()=>{const value=await info(relay);assert(value.p2p_addresses.length);return lan ? lan.relayAddress(value.peer_id) : value.p2p_addresses[0];},"relay address");
 }
 const nodes=[];
 for(let i=0;i<3;i++){
  const node=await startNode(`nas-${i}`,relayAddress);await login(node,i);nodes.push(node);
  const nas=await info(node);node.peer=nas.peer_id;
  const key=(await request(node,"content/key")).body;
  const hash=await wallets[i].writeContract({address:registry.address,abi:registry.abi,functionName:"register",args:[nas.peer_id,key.public_key,`User ${i}`]});await client.waitForTransactionReceipt({hash});
  if(!lan?.direct)await until(async()=>assert((await info(node)).p2p_addresses.some(address=>address.includes("p2p-circuit"))),`relay reservation ${i}`);
 }
 if(lan?.direct)await lan.connect(nodes,stop);
 console.log(lan?.direct ? "Three NAS nodes configured direct LAN connections." : "Three NAS nodes reserved relay circuits.");
 const [a,b,c]=nodes;
 async function action(node,action){
  const prepared=await request(node,"social/prepare",{action});assert.equal(prepared.status,200,JSON.stringify(prepared.body));const value=prepared.body,intent=value.intent;
  assert(actionsMatch(intent.action,action));
  const typedAction=action.action==="publish"?{action:"publish",post:{...intent.action.post,recipients:intent.action.post.recipients.map(v=>new Uint8Array(v))}}:action.action==="follow"?{...intent.action,target:new Uint8Array(intent.action.target)}:{...intent.action,post_id:new Uint8Array(intent.action.post_id)};
  const encoded=encodeCanonical({...intent,scope:{...intent.scope,registry_contract:new Uint8Array(intent.scope.registry_contract)},actor:new Uint8Array(intent.actor),nonce:new Uint8Array(intent.nonce),action:typedAction});assert.equal(bytesToHex(encoded),value.payload);assert.equal(keccak256(encoded),value.payload_hash);
  const signature=await wallets[node.index].signTypedData({domain:{name:"RedLeaves",version:"1",chainId:84532,verifyingContract:registry.address},types:{SocialAction:[{name:"payloadHash",type:"bytes32"}]},primaryType:"SocialAction",message:{payloadHash:value.payload_hash}});
  const committed=await request(node,"social/commit",{payload:value.payload,signature});assert.equal(committed.status,200,JSON.stringify(committed.body));return committed.body;
 }
 const walletBytes=node=>Array.from(Buffer.from(node.owner.slice(2),"hex"));
 async function follow(node,target,enabled=true){return action(node,{action:"follow",target:walletBytes(target),enabled});}
 async function publish(distribution="broadcast",targets=[],extra={}){return action(a,{action:"publish",post:{title:`Delivery ${distribution}`,body:"Signed content over a relay circuit",distribution,recipients:targets.map(walletBytes),...extra}});}
 async function feed(node){const response=await request(node,"posts");assert.equal(response.status,200,JSON.stringify(response.body));return response.body.posts;}
 async function has(node,id){assert((await feed(node)).some(post=>post.id===id));}
 async function absent(node,id){assert(!(await feed(node)).some(post=>post.id===id));}
 const prefs=(node,value)=>request(node,"social/preferences",value);
 await follow(b,a);await follow(a,b);
 await until(async()=>{const rel=(await request(a,"social/relationships")).body;assert(rel.followers.includes(b.owner));assert(rel.following.includes(b.owner));},"cross-NAS mutual follows");
 await prefs(c,{broadcasts:true,following:false,followers:false,direct:true});
 const sseAbort=new AbortController();const sse=await fetch(`${b.api}/api/social/events`,{headers:{Authorization:`Bearer ${b.token}`},signal:sseAbort.signal});assert.equal(sse.status,200);const reader=sse.body.getReader();await reader.read();
 console.log("Cross-NAS follow synchronization passed.");
 const publicPost=await publish();
 await until(()=>has(b,publicPost.id),"public reception by following");await until(()=>has(c,publicPost.id),"public reception by broadcast preference");
 const update=await Promise.race([reader.read(),pause(10000).then(()=>{throw new Error("Missing live inbox update");})]);assert(new TextDecoder().decode(update.value).includes("event: update"));sseAbort.abort();await reader.cancel().catch(()=>{});
 console.log("Public delivery and live SSE passed.");
 const postBytes=id=>Array.from(Buffer.from(id,"hex"));
 async function likes(node,id){const result=await request(node,`posts/${id}/likes`);assert.equal(result.status,200,JSON.stringify(result.body));return result.body;}
 async function like(node,id,enabled){return action(node,{action:"like",post_id:postBytes(id),enabled});}
 assert.equal((await likes(b,publicPost.id)).count,0);
 await like(b,publicPost.id,true);
 await until(async()=>{assert.equal((await likes(a,publicPost.id)).count,1);assert.equal((await likes(b,publicPost.id)).liked,true);assert.equal((await likes(c,publicPost.id)).count,1);},"authoritative cross-NAS like");
 await like(b,publicPost.id,false);
 await until(async()=>{assert.equal((await likes(a,publicPost.id)).count,0);assert.equal((await likes(b,publicPost.id)).liked,false);},"cross-NAS unlike");
 console.log("Author aggregation and on-demand like pulls passed.");
 for(const scope of ["following","followers","friends"]){const post=await publish(scope,[b]);await until(()=>has(b,post.id),scope);await absent(c,post.id);assert.equal((await request(c,`posts/${post.id}`)).status,404);}
 const direct=await publish("directed",[c]);await until(()=>has(c,direct.id),"explicit recipient");await absent(b,direct.id);
 await like(c,direct.id,true);await until(async()=>assert.equal((await likes(a,direct.id)).count,1),"directed like");
 assert.equal((await request(b,`posts/${direct.id}/likes`)).status,404);
 await request(a,`posts/${direct.id}/likes/access`,{target:c.owner,allowed:false});
 assert.equal((await request(c,`posts/${direct.id}/likes`)).status,404);
 assert.equal((await request(c,`posts/${direct.id}`)).status,200);
 await request(a,`posts/${direct.id}/likes/access`,{target:c.owner,allowed:true});
 assert.equal((await likes(c,direct.id)).count,1);
 await request(a,"social/blocks",{target:c.owner,blocked:true});
 assert.equal((await request(c,`posts/${direct.id}/likes`)).status,404);
 await request(a,"social/blocks",{target:c.owner,blocked:false});
 assert.equal((await likes(c,direct.id)).count,1);
 console.log("Latest author permissions and revoked like access passed.");
 const blocked=await request(c,"social/blocks",{target:a.owner,blocked:true});assert.equal(blocked.status,200);
 const hidden=await publish();await until(()=>has(b,hidden.id),"unblocked receiver");await pause(1000);await absent(c,hidden.id);
 await request(c,"social/blocks",{target:a.owner,blocked:false});
 await prefs(b,{broadcasts:false,following:false,followers:false,direct:false});
 const filtered=await publish("directed",[b]);
 await until(async()=>{const progress=(await request(a,`posts/${filtered.id}/delivery`)).body;assert.equal(progress.targets[0].state,"filtered");},"receiver preference filter");await absent(b,filtered.id);
 await prefs(b,{broadcasts:false,following:true,followers:false,direct:true});
 console.log("Scope, preference and block filters passed.");
 const image=Buffer.concat([Buffer.from("89504e470d0a1a0a","hex"),Buffer.alloc(300000,42)]);
 const uploadResponse=await fetch(`${a.api}/api/media`,{method:"POST",headers:{"Content-Type":"image/png",Authorization:`Bearer ${a.token}`},body:image});assert.equal(uploadResponse.status,200);const media=await uploadResponse.json();
 const photo=await publish("directed",[b],{body:"",media:[media]});
 await until(()=>has(b,photo.id),"directed photo");
 const content=await request(b,`posts/${photo.id}`);assert.equal(content.status,200,JSON.stringify(content.body));assert.equal(content.body.intent.action.post.media[0].digest,media.digest);assert.equal(content.body.intent.action.post.price_wei,undefined);
 const mediaResponse=await fetch(`${b.api}/api/posts/${photo.id}/media/${media.id}`,{headers:{Authorization:`Bearer ${b.token}`}});assert.equal(mediaResponse.status,200,await mediaResponse.clone().text());assert.deepEqual(Buffer.from(await mediaResponse.arrayBuffer()),image);
 assert.equal((await request(c,`posts/${photo.id}`)).status,404);
 console.log("Directed content and encrypted media passed.");
 async function access(id,level,recipients=[]){const result=await request(a,`posts/${id}/access`,{level,recipients:recipients.map(node=>node.owner)});assert.equal(result.status,200,JSON.stringify(result.body));}
 const restricted=await publish("directed",[b]);await until(()=>has(b,restricted.id),"initial cached private copy");
 await access(restricted.id,"none");
 assert.equal((await request(b,`posts/${restricted.id}`)).status,200,"existing cache must remain readable");
 assert.equal((await request(b,`posts/${restricted.id}/likes`)).status,404,"cached body must not authorize interaction pulls");

 assert.equal((await request(c,`posts/${restricted.id}?author=${a.owner}`)).status,404,"new fetch must be denied");
 await access(restricted.id,"followers");assert.equal((await request(b,`posts/${restricted.id}/likes`)).status,200);assert.equal((await request(c,`posts/${restricted.id}?author=${a.owner}`)).status,404);
 await access(restricted.id,"following");assert.equal((await request(b,`posts/${restricted.id}/likes`)).status,200);assert.equal((await request(c,`posts/${restricted.id}?author=${a.owner}`)).status,404);
 await access(restricted.id,"directed",[c]);
 assert.equal((await request(c,`posts/${restricted.id}?author=${a.owner}`)).status,200,"new recipient can fetch beyond original signed delivery list");
 assert.equal((await request(c,`posts/${restricted.id}/likes`)).status,200);assert.equal((await request(b,`posts/${restricted.id}/likes`)).status,404);
 assert.equal((await request(b,`posts/${restricted.id}`)).status,200);
 await access(restricted.id,"public");assert.equal((await request(b,`posts/${restricted.id}/likes`)).status,200);
 const expanded=await publish("directed",[b]);await until(()=>has(b,expanded.id),"second private copy");await access(expanded.id,"public");await until(()=>has(c,expanded.id),"formerly encrypted post becomes public");
 await access(photo.id,"none");
 assert.equal((await request(b,`posts/${photo.id}`)).status,200);
 assert.equal((await fetch(`${b.api}/api/posts/${photo.id}/media/${media.id}`,{headers:{Authorization:`Bearer ${b.token}`}})).status,404);
 console.log("All five access levels passed; existing caches survive while new fetches, media and likes use current permissions.");

 await stop(b.process);
 const offline=await publish("directed",[b]);
 await until(async()=>{const progress=(await request(a,`posts/${offline.id}/delivery`)).body;assert(progress.targets[0].attempts>0);assert.equal(progress.targets[0].state,"retrying");},"offline retry");
 await stop(a.process);await a.restart();await b.restart();
 await until(async()=>{assert.equal((await info(a)).peer_id,a.peer);assert.equal((await info(b)).peer_id,b.peer);await has(b,offline.id);},"restart delivery",70);
 await until(async()=>{const progress=(await request(a,`posts/${offline.id}/delivery`)).body;assert.equal(progress.targets[0].state,"delivered");},"receiver save acknowledgment");
 assert.equal((await likes(b,offline.id)).count,0);
 await stop(a.process);
 await like(b,offline.id,true);
 assert.equal((await request(b,`posts/${offline.id}/likes`)).status,503);
 await stop(b.process);await b.restart();
 await a.restart();
 await until(async()=>{assert.equal((await likes(a,offline.id)).count,1);const value=await likes(b,offline.id);assert.equal(value.count,1);assert.equal(value.liked,true);assert.equal(value.pending,null);},"durable offline like synchronization",90);
 console.log("Offline like queue survives restart; no stale statistics fallback.");
 await stop(a.process);
 const cached=await request(b,`posts/${offline.id}`);assert.equal(cached.status,200);assert.equal(cached.body.intent.action.post.body,"Signed content over a relay circuit");
 console.log(`Delivery integration passed: ${lan?.direct ? "direct LAN connections" : "relay circuits"}, all scopes, follow synchronization, author-aggregated likes with current permissions and offline recovery, live SSE, reception/block filters, audience isolation, encrypted media chunks, offline retry, durable restart and offline cached reading.`);
 if(lan)await lan.report();
} catch(error) {
 for(const process of children){if(process.log) console.error(process.log.slice(-3000));}
 throw error;
} finally {
 for(const process of children.reverse()){if(process.exitCode===null)await stop(process).catch(error=>console.error(`Unable to stop test process: ${error.message}`));}
 if(lan)await lan.cleanup();
 await pause(300);await rm(directory,{recursive:true,force:true});
}
