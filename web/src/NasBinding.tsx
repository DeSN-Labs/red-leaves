import React, { useEffect, useRef, useState } from "react";
import type { Hex } from "viem";
import { createRegistry, registryAddress, type Registration } from "./registry";
import { getNasInfo, nasApi, getWalletProvider } from "./wallet";

export function NasBinding({ owner }: { owner: string }) {
  const [peerId, setPeerId] = useState("");
  const [record, setRecord] = useState<Registration | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [editing, setEditing] = useState(false);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  const [nickname, setNickname] = useState("");
  const [publicKey, setPublicKey] = useState("");
  const [keyAcknowledged, setKeyAcknowledged] = useState(false);
  const [busy, setBusy] = useState(false);
  const [hash, setHash] = useState<Hex | null>(null);
  const generation = useRef(0);
  const gateway = () => {
    return createRegistry(getWalletProvider(), registryAddress, owner, localStorage);
  };
  const explain = (cause: unknown) => {
    const code = (cause as { code?: number; cause?: { code?: number } })?.code
      ?? (cause as { cause?: { code?: number } })?.cause?.code;
    if (code === 4001) return "You canceled the wallet request. Your NAS login is still active.";
    if (cause instanceof Error && !cause.message.includes("Request Arguments:")) return cause.message;
    return "The registry request failed. Check your network and transaction status before retrying.";
  };
  async function load(current: number) {
    setBusy(true); setLoaded(false); setPeerId(""); setError(""); setMessage("");
    try {
      const results = await Promise.allSettled([getNasInfo(), Promise.resolve().then(() => {
        const registry = gateway();
        const pending = registry.pending();
        if (generation.current === current) setHash(pending?.hash ?? null);
        return registry.read();
      })]);
      if (generation.current !== current) return;
      const errors: string[] = [];
      if (results[0].status === "fulfilled") setPeerId(results[0].value.peer_id);
      else errors.push(explain(results[0].reason));
      if (results[1].status === "fulfilled") {
        setRecord(results[1].value); setLoaded(true);
        if (results[1].value) setEditing(false);
      } else errors.push(explain(results[1].reason));
      setError(errors.join(" "));

    } catch (cause) {
      if (generation.current === current) setError(explain(cause));
    } finally { if (generation.current === current) setBusy(false); }
  }
  useEffect(() => {
    const current = ++generation.current;
    void load(current);
    return () => { generation.current++; };
  }, [owner]);

  const onHash = (current: number) => (value: Hex) => {
    if (generation.current === current) { setHash(value); setMessage("Transaction submitted. Waiting for two block confirmations…"); }
  };
  async function submit(resume = false) {
    const current = generation.current;
    setBusy(true); setError(""); setMessage("");
    try {
      const registry = gateway();
      let result: Registration;
      if (resume) result = await registry.resume(onHash(current));
      else {
        if (!keyAcknowledged) throw new Error("Confirm that you trust this NAS to retain the matching private key.");
        // Recheck the authenticated NAS identity immediately before preparing the transaction.
        const nas = await getNasInfo();
        if (nas.peer_id !== peerId) throw new Error("The NAS PeerId changed. Refresh the registration details before continuing.");
        if (generation.current !== current) return;
        setMessage("Confirm the registration transaction and gas fee in your wallet.");
        result = await registry.register({ nasPeerId: peerId, encryptionPublicKey: publicKey as Hex, nickname }, onHash(current), () => generation.current === current);
      }
      if (generation.current === current) {
        setRecord(result); setLoaded(true); setHash(null); setEditing(false);
        setMessage("On-chain registration confirmed with two blocks. Your submitted fields have been verified.");
      }
    } catch (cause) {
      if (generation.current === current) {
        setError(explain(cause)); setMessage("");
        try { setHash(gateway().pending()?.hash ?? null); } catch { /* Keep the known transaction hash visible. */ }
      }
    } finally { if (generation.current === current) setBusy(false); }
  }
  async function generateKey() {
    const current = generation.current;
    setBusy(true); setError("");
    try {
      const { public_key: value } = await nasApi<{ public_key: string }>("content/key");
      if (generation.current === current) {
        setPublicKey(value); setKeyAcknowledged(false);
        setMessage("Your NAS holds the matching private key. Back up the NAS master key and user key files together. No private key was sent to this browser.");
      }
    } catch { if (generation.current === current) setError("Unable to prepare the NAS encryption key. Check the NAS key storage. No transaction was sent."); }
    finally { if (generation.current === current) setBusy(false); }
  }

  return <section className={`nas-binding ${editing || detailsOpen ? "expanded" : "compact"}`} aria-label="Profile registration">
    <div className="registration-heading">
      <div>
        <span className={`registration-badge ${loaded && record ? "registered" : ""}`}>
          {hash ? "Transaction pending" : !loaded ? busy ? "Checking registration…" : "Status unavailable" : record ? "Registered" : "Not registered"}
        </span>
        <h3>{loaded && !record && !hash ? "Complete your on-chain profile" : record && loaded ? `Profile: ${record.nickname}` : "Your on-chain profile"}</h3>
        <p>{loaded && !record && !hash
          ? "Your wallet has not registered a profile on Base Sepolia. Choose your nickname and encryption public key when you are ready."
          : record && loaded ? "Your wallet has a registered profile. Signing in does not submit or change it."
          : "Check your profile registration before creating a new one. Your NAS login remains active."}</p>
      </div>
      <div className="binding-actions">
        {loaded && !record && !hash && !editing && <button className="button primary" disabled={busy || !peerId} onClick={() => setEditing(true)}>Register profile</button>}
        {hash && <button className="button primary" disabled={busy} onClick={() => void submit(true)}>Check pending transaction</button>}
        <button className="button demo" disabled={busy} onClick={() => void load(generation.current)}>{busy ? "Checking…" : "Refresh status"}</button>
        {!editing && <button className="button demo" onClick={() => setDetailsOpen(value => !value)}>{detailsOpen ? "Hide details" : "View details"}</button>}
      </div>
    </div>
    {error && <p className="error" role="alert">{error}</p>}
    {message && <p role="status">{message}</p>}
    {loaded && !record && !peerId && <p role="status">Registration requires an authenticated NAS identity. Check the NAS server and retry; demo and relay sessions cannot register here.</p>}
    {hash && <p className="transaction-hash">Transaction: <a href={`https://sepolia.basescan.org/tx/${hash}`} target="_blank" rel="noreferrer">{hash}</a></p>}
    {(detailsOpen || editing) && <div className="registration-details">
      <dl><dt>Network</dt><dd>Base Sepolia · 84532</dd>
        <dt>Registry</dt><dd>{registryAddress || "Not configured"}</dd>
        <dt>Wallet</dt><dd>{owner}</dd>
        <dt>Current NAS PeerId</dt><dd>{peerId || "Not available"}</dd></dl>
      <p className="muted">The NAS identity persists across restarts. Remote delivery requires reachable peers or a configured relay.</p>
      {loaded && record && <div>
        <dl><dt>Registered NAS PeerId</dt><dd>{record.nasPeerId}</dd>
          <dt>Encryption public key</dt><dd>{record.encryptionPublicKey}</dd>
          <dt>Nickname</dt><dd>{record.nickname}</dd></dl>
        <p>{!peerId ? "The current NAS identity is unavailable. Refresh status before comparing this binding." : record.nasPeerId === peerId ? "The registered NAS matches this server." : "This wallet is registered to a different NAS. No automatic rebinding will be performed."}</p>
        <p className="muted">Viewing this record does not prove that this browser holds its encryption private key.</p>
      </div>}
    </div>}
    {editing && loaded && !record && !hash && peerId && <form aria-label="Register profile" onSubmit={event => { event.preventDefault(); void submit(); }}>
      <h4>Register your profile</h4>
      <label>Nickname<input autoFocus required value={nickname} disabled={busy} onChange={event => setNickname(event.target.value)} autoComplete="nickname" placeholder="Choose your display name" /></label>
      <label>Encryption public key<input required readOnly value={publicKey} disabled={busy} placeholder="Prepare your NAS public key below" spellCheck={false} /></label>
      <p>Use your NAS-held X25519 key to receive and decrypt content while your browser is offline. Existing browser keys cannot be exported or migrated automatically.</p>
      <button type="button" className="button demo" disabled={busy} onClick={() => void generateKey()}>Prepare NAS encryption public key</button>
      <label className="key-acknowledgement"><input type="checkbox" required checked={keyAcknowledged} disabled={busy} onChange={event => setKeyAcknowledged(event.target.checked)} />I trust this NAS to hold my content decryption key and understand that NAS key files require backup.</label>
      <p>Your wallet address, NAS PeerId, encryption public key and nickname will be public on-chain. Private keys, avatar and gender are not submitted. Your wallet will ask you to confirm the transaction and gas fee.</p>
      <div className="binding-actions">
        <button className="button primary" disabled={busy || !keyAcknowledged} type="submit">{busy ? "Processing…" : "Confirm and register on Base Sepolia"}</button>
        <button className="button demo" type="button" disabled={busy} onClick={() => setEditing(false)}>Not now</button>
      </div>
    </form>}
  </section>;
}
