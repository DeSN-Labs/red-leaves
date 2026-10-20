import test from "node:test";
import assert from "node:assert/strict";
import { indexedDB } from "fake-indexeddb";
import { browserEncryptionKey } from "../src/encryption-key.ts";

globalThis.indexedDB=indexedDB;
async function stored(owner) {
  const database=await new Promise((resolve,reject)=>{
    const request=indexedDB.open("red-leaves-encryption-keys",1);
    request.onsuccess=()=>resolve(request.result);request.onerror=()=>reject(request.error);
  });
  try {
    return await new Promise((resolve,reject)=>{
      const request=database.transaction("keys").objectStore("keys").get(`x25519:${owner.toLowerCase()}`);
      request.onsuccess=()=>resolve(request.result);request.onerror=()=>reject(request.error);
    });
  } finally {database.close();}
}

test("browser keys persist, remain non-extractable and match their public key",async()=>{
  const alice=`0x${"11".repeat(20)}`,bob=`0x${"22".repeat(20)}`;
  const publicAlice=await browserEncryptionKey(alice);
  const publicBob=await browserEncryptionKey(bob);
  assert.match(publicAlice,/^0x[0-9a-f]{64}$/);
  assert.notEqual(publicAlice,publicBob);
  assert.equal(await browserEncryptionKey(alice.toUpperCase()),publicAlice);
  const keyA=await stored(alice),keyB=await stored(bob);
  assert.equal(keyA.privateKey.extractable,false);
  await assert.rejects(crypto.subtle.exportKey("pkcs8",keyA.privateKey));
  const importPublic=hex=>crypto.subtle.importKey("raw",Buffer.from(hex.slice(2),"hex"),"X25519",false,[]);
  const sharedA=await crypto.subtle.deriveBits({name:"X25519",public:await importPublic(publicBob)},keyA.privateKey,256);
  const sharedB=await crypto.subtle.deriveBits({name:"X25519",public:await importPublic(publicAlice)},keyB.privateKey,256);
  assert.deepEqual(new Uint8Array(sharedA),new Uint8Array(sharedB));
});

test("concurrent browser key creation preserves one identity without overwriting",async()=>{
  const owner=`0x${"33".repeat(20)}`;
  const [first,second]=await Promise.all([browserEncryptionKey(owner),browserEncryptionKey(owner)]);
  assert.equal(first,second);assert.equal((await stored(owner)).publicKey,first);
});
