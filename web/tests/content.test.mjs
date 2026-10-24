import test from "node:test";
import assert from "node:assert/strict";
import { encodeCanonical, addressBytes, audienceWallets, actionsMatch } from "../src/content.ts";

test("content signing CBOR matches the protocol canonical reference and ignores object insertion order", () => {
  assert.equal(Buffer.from(encodeCanonical({ z: true, a: 1 })).toString("hex"), "a2616101617af5");
  assert.equal(Buffer.from(encodeCanonical({ a: new Uint8Array([1,2]), b: null })).toString("hex"), "a261614201026162f6");
  assert.equal(Buffer.from(encodeCanonical({ a: 84532 })).toString("hex"), "a161611a00014a34");
  for (const value of [-1, NaN, Infinity, 1.5, Number.MAX_SAFE_INTEGER + 1]) assert.throws(() => encodeCanonical(value), /canonical integer/);
});
test("direct recipients use exact nonzero 20-byte wallet addresses", () => {
  assert.deepEqual(addressBytes("0x"+"12".repeat(20)),Array(20).fill(0x12));
  for (const value of ["0x"+"00".repeat(20),"0x12","12".repeat(20),"0x"+"gg".repeat(20)]) assert.throws(() => addressBytes(value), /wallet address/);
});

test("distribution scopes resolve mutual friends and deduplicate explicit selections", () => {
 const relationships = { following: ["0xCC", "0xAA"], followers: ["0xaa", "0xbb"] };
 assert.deepEqual(audienceWallets("broadcast", relationships), []);
 assert.deepEqual(audienceWallets("following", relationships), ["0xaa", "0xcc"]);
 assert.deepEqual(audienceWallets("followers", relationships), ["0xaa", "0xbb"]);
 assert.deepEqual(audienceWallets("friends", relationships), ["0xaa"]);
 assert.deepEqual(audienceWallets("directed", relationships, ["0xCC", "0xcc", "0xAA"]), ["0xaa", "0xcc"]);
});

test("reviewed media metadata compares independently of JSON key order without ignoring changes", () => {
 const a = { action: "publish", post: { title: "Photo", body: "", distribution: "broadcast", recipients: [], media: [{ id: "ab", mime: "image/png", digest: "1234", size: 123 }] } };
 const b = { action: "publish", post: { ...a.post, media: [{ digest: "1234", id: "ab", size: 123, mime: "image/png" }] } };
 assert(actionsMatch(a,b));
 b.post.media[0].size++;
 assert(!actionsMatch(a,b));
 b.post.media[0].size--;b.post.media[0].unexpected=true;
 assert(!actionsMatch(a,b));
});

test("reviewed publications cannot gain pricing fields", () => {
 const action = { action: "publish", post: { title: "Post", body: "Text", distribution: "broadcast", recipients: [] } };
 assert(!actionsMatch({ ...action, post: { ...action.post, price_wei: "0" } }, action));
});

test("like review binds the complete post multihash and the enabled flag", () => {
  const post_id = [0x12,0x20,...Array(32).fill(7)];
  const action = { action: "like", post_id, enabled: true };
  assert(actionsMatch({ enabled: true, post_id: [...post_id], action: "like" }, action));
  assert(!actionsMatch({ ...action, enabled: false }, action));
  assert(!actionsMatch({ ...action, post_id: [...post_id.slice(0,-1),8] }, action));
  const canonical = encodeCanonical({ ...action, post_id: new Uint8Array(post_id) });
  assert(Buffer.from(canonical).includes(Buffer.from([0x58,0x22,0x12,0x20])));
});
