import test from "node:test";
import assert from "node:assert/strict";
import { encodeCanonical, addressBytes, priceEthToWei, audienceWallets, actionsMatch } from "../src/content.ts";

test("content signing CBOR matches the protocol canonical reference and ignores object insertion order", () => {
  assert.equal(Buffer.from(encodeCanonical({ z: true, a: 1 })).toString("hex"), "a2616101617af5");
  assert.equal(Buffer.from(encodeCanonical({ a: new Uint8Array([1,2]), b: null })).toString("hex"), "a261614201026162f6");
  assert.equal(Buffer.from(encodeCanonical({ a: 84532 })).toString("hex"), "a161611a00014a34");
  for (const value of [-1, NaN, Infinity, 1.5, Number.MAX_SAFE_INTEGER + 1]) assert.throws(() => encodeCanonical(value), /canonical integer/);
});
test("paid prices cannot silently round or use ambiguous numeric syntax", () => {
  assert.equal(priceEthToWei("0.001"), "1000000000000000");
  assert.equal(priceEthToWei("0.000000000000000001"), "1");
  for (const value of ["0", "-1", "1e-3", "0.0000000000000000001", " 1", "1,000", "01"]) assert.throws(() => priceEthToWei(value));
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
 const a = { action: "publish", post: { title: "Photo", body: "", distribution: "broadcast", recipients: [], price_wei: "0", media: [{ id: "ab", mime: "image/png", digest: "1234", size: 123 }] } };
 const b = { action: "publish", post: { ...a.post, media: [{ digest: "1234", id: "ab", size: 123, mime: "image/png" }] } };
 assert(actionsMatch(a,b));
 b.post.media[0].size++;
 assert(!actionsMatch(a,b));
 b.post.media[0].size--;b.post.media[0].unexpected=true;
 assert(!actionsMatch(a,b));
});
