import React, { useState } from "react";
import type { Hex } from "viem";
import { NasBinding } from "./NasBinding";
import { nasApi } from "./wallet";
import { updateNasKey } from "./content";

export function Profile({ owner }: { owner: string }) {
  const [key, setKey] = useState<Hex | null>(null), [hash, setHash] = useState<Hex | null>(null);
  const [busy, setBusy] = useState(false), [message, setMessage] = useState("");
  const [error, setError] = useState(""), [revision, setRevision] = useState(0);
  async function run(task: () => Promise<void>) {
    setBusy(true); setError(""); setMessage("");
    try { await task(); } catch (cause) { setError(cause instanceof Error ? cause.message.slice(0, 500) : "Profile request failed."); }
    finally { setBusy(false); }
  }
  return <div className="profile-page">
    <section className="content-panel"><span className="registration-badge">Personal profile</span><h2>My wallet &amp; profile</h2><p className="key-value">{owner}</p><p>Manage your nickname, NAS binding and encryption public key here. Your application private key stays on your trusted NAS.</p></section>
    <NasBinding key={revision} owner={owner} />
    <section className="content-panel"><h2>NAS encryption key</h2>
      <p>If your on-chain profile uses an older browser-held key, update it to the NAS-held key before publishing. This does not recover old encrypted content.</p>
      {error && <p className="error" role="alert">{error}</p>}{message && <p role="status">{message}</p>}
      <button className="button demo" disabled={busy} onClick={() => void run(async () => { setKey((await nasApi<{ public_key: Hex }>("content/key")).public_key); })}>Show NAS public key</button>
      {key && <><p className="key-value">{key}</p><button className="button primary" disabled={busy} onClick={() => void run(async () => { await updateNasKey(owner, key, setHash); setRevision(value => value + 1); setMessage("NAS encryption public key updated on-chain."); })}>Confirm key update on-chain</button></>}
      {hash && <p>Key transaction: <a href={`https://sepolia.basescan.org/tx/${hash}`} target="_blank" rel="noreferrer">{hash}</a></p>}
    </section>
  </div>;
}
