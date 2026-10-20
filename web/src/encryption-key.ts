import type { Hex } from "viem";

type StoredKey = { publicKey: Hex; privateKey: CryptoKey };
// Non-extractable private keys remain in the browser. No key material is sent to the NAS.
export async function browserEncryptionKey(owner: string): Promise<Hex> {
  if (!globalThis.crypto?.subtle || !globalThis.indexedDB) throw new Error("This browser cannot securely store an encryption key. Use a supported browser or supply your own public key.");
  const database = await new Promise<IDBDatabase>((resolve, reject) => {
    const request = indexedDB.open("red-leaves-encryption-keys", 1);
    let blocked = false;
    request.onupgradeneeded = () => request.result.createObjectStore("keys");
    request.onsuccess = () => { if (blocked) request.result.close(); else resolve(request.result); };
    request.onerror = () => reject(new Error("Unable to open the browser key store."));
    request.onblocked = () => { blocked = true; reject(new Error("The browser key store is blocked by another tab.")); };
  });
  const id = `x25519:${owner.toLowerCase()}`;
  const access = (candidate?: StoredKey) => new Promise<StoredKey | undefined>((resolve, reject) => {
    const tx = database.transaction("keys", candidate ? "readwrite" : "readonly");
    const store = tx.objectStore("keys");
    const request = store.get(id);
    let value: StoredKey | undefined;
    request.onsuccess = () => {
      value = request.result;
      if (!value && candidate) { value = candidate; store.add(candidate, id); }
    };
    tx.oncomplete = () => resolve(value);
    tx.onerror = tx.onabort = () => reject(new Error("Unable to persist the browser encryption key. Registration has not been submitted."));
  });
  try {
    let stored = await access();
    if (!stored) {
      const pair = await crypto.subtle.generateKey({ name: "X25519" }, false, ["deriveBits"]) as CryptoKeyPair;
      const raw = new Uint8Array(await crypto.subtle.exportKey("raw", pair.publicKey));
      const publicKey = `0x${Array.from(raw, byte => byte.toString(16).padStart(2, "0")).join("")}` as Hex;
      stored = await access({ publicKey, privateKey: pair.privateKey });
    }
    if (!stored || stored.privateKey.type !== "private" || stored.privateKey.extractable || stored.privateKey.algorithm.name !== "X25519") throw new Error("The stored browser key is invalid. It will not be replaced automatically.");
    return stored.publicKey;
  } finally { database.close(); }
}
