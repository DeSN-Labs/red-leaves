import React, { useEffect, useRef, useState } from "react";
import { formatEther } from "viem";
import { ImagePlus, Video, FileText, X, LockKeyhole, ChevronLeft, ChevronRight } from "lucide-react";
import { nasApi, nasBlob, nasUpload, shortAddress, watchNasChanges } from "./wallet";
import { addressBytes, audienceWallets, priceEthToWei, submitAction, listPaidPost, buyPost, type Draft, type Distribution, type Media, type Relationships, type PostSummary, type PostDetail, type PostStatus, type Preferences } from "./content";

const scopes: Record<Distribution, string> = { broadcast: "Network broadcast", following: "People I follow", followers: "People who follow me", friends: "Friends (mutual follows)", directed: "Specific people" };

// Fetch with the Bearer session; object URLs are revoked when the view or session changes.
function MediaView({ file, path, controls = false }: { file: Media; path: string; controls?: boolean }) {
  const [url, setUrl] = useState(""), [error, setError] = useState("");
  const container = useRef<HTMLDivElement>(null);
  useEffect(() => {
    let active = true, objectUrl = "";
    const controller = new AbortController();
    setUrl(""); setError("");
    const load = () => { void nasBlob(path, controller.signal).then(blob => { if (active) { objectUrl = URL.createObjectURL(blob); setUrl(objectUrl); } }).catch(cause => { if (active) setError(explain(cause)); }); };
    const observer = new IntersectionObserver(entries => { if (entries.some(entry => entry.isIntersecting)) { observer.disconnect(); load(); } }, { rootMargin: "150px" });
    if (container.current) observer.observe(container.current);
    return () => { active = false; controller.abort(); observer.disconnect(); if (objectUrl) URL.revokeObjectURL(objectUrl); };
  }, [path]);
  return <div className="media-view" ref={container}>{error ? <p className="media-error" role="alert">{error}</p> : !url ? <span className="media-loading" role="status">Loading media…</span> : file.mime.startsWith("video/") ? <video src={url} controls={controls} preload="metadata" playsInline aria-label="Post video" /> : <img src={url} alt="Post attachment" loading="lazy" />}</div>;
}
function Detail({ post, value, busy, error, close, unlock }: { post: PostSummary; value: PostDetail; busy: boolean; error: string; close: () => void; unlock: (purchase: boolean) => void }) {
  const dialog = useRef<HTMLDialogElement>(null), [index, setIndex] = useState(0);
  const draft = value.intent?.action.post, files = draft?.media ?? [];
  useEffect(() => { const node = dialog.current; node?.showModal(); return () => { node?.close(); }; }, []);
  return <dialog className="note-dialog" ref={dialog} onCancel={close} onClick={event => { if (event.target === event.currentTarget) close(); }} aria-label="Post details">
    <button className="note-close icon-button" aria-label="Close post" onClick={close}><X size={22} /></button>
    <div className={`note-detail ${files.length ? "has-media" : ""}`}>
      {files.length > 0 && <div className="note-gallery"><MediaView key={files[index].id} file={files[index]} path={`posts/${post.id}/media/${files[index].id}`} controls />
        {files.length > 1 && <div className="gallery-navigation"><button className="icon-button" aria-label="Previous image" disabled={index === 0} onClick={() => setIndex(index - 1)}><ChevronLeft /></button><span>{index + 1} / {files.length}</span><button className="icon-button" aria-label="Next image" disabled={index === files.length - 1} onClick={() => setIndex(index + 1)}><ChevronRight /></button></div>}
      </div>}
      <div className="note-copy"><p className="note-author">{shortAddress(post.author)}</p><span className="registration-badge">{scopes[post.distribution as Distribution] ?? post.distribution}</span><h2>{draft?.title ?? post.title}</h2>
        {error && <p className="error" role="alert">{error}</p>}
        {draft && <p className="post-body">{draft.body}</p>}
        {value.locked && <div className="locked-post"><LockKeyhole size={30} /><h3>Unlock this post</h3><p>The content and media are available after confirmed payment.</p><button className="button primary" disabled={busy} onClick={() => unlock(true)}>{busy ? "Processing…" : `Pay ${formatEther(BigInt(post.price_wei))} test ETH & unlock`}</button><button className="button demo" disabled={busy} onClick={() => unlock(false)}>Check existing payment</button></div>}
        <time>{new Date(post.created_at * 1000).toLocaleString()}</time>
      </div>
    </div>
  </dialog>;
}
export function Publishing({ owner, page }: { owner: string; page: "publish" | "feed" | "relationships" }) {
  const [title, setTitle] = useState(""), [body, setBody] = useState("");
  const [mode, setMode] = useState<"text" | "images" | "video">("text"), [media, setMedia] = useState<Media[]>([]);
  const [pending, setPending] = useState<Media[]>([]);
  const [paid, setPaid] = useState(false), [price, setPrice] = useState("0.001");
  const [distribution, setDistribution] = useState<Distribution>("broadcast");
  const [recipients, setRecipients] = useState(""), [selectedPeople, setSelectedPeople] = useState<string[]>([]), [peopleTab, setPeopleTab] = useState<"friends" | "following">("friends");
  const [confirmed, setConfirmed] = useState(false);
  const [prefs, setPrefs] = useState<Preferences>({ broadcasts: false, following: true, followers: false, direct: true });
  const [items, setItems] = useState<PostSummary[]>([]), [selected, setSelected] = useState<PostSummary | null>(null), [detail, setDetail] = useState<PostDetail | null>(null);
  const [relations, setRelations] = useState<Relationships>({ following: [], followers: [] });
  const [target, setTarget] = useState(""), [busy, setBusy] = useState(false), [error, setError] = useState(""), [message, setMessage] = useState("");
  const generation = useRef(0), relationshipSnapshot = useRef("");
  const [networkStatus, setNetworkStatus] = useState("checking"), [blocked, setBlocked] = useState<string[]>([]);
  const [progress, setProgress] = useState<Record<string, { target: string; state: string; attempts: number; last_error: string | null }[]>>({});
  const friends = audienceWallets("friends", relations);
  const audience = audienceWallets(distribution, relations, [...selectedPeople, ...recipients.split(/[\s,]+/).filter(Boolean)]);
  async function refresh(current: number, live = false) {
    const [preferences, posts, relationships, uploads] = await Promise.all([
      nasApi<Preferences>("social/preferences"), nasApi<{ posts: PostSummary[]; network_delivery: string }>(`posts?mine=${page === "publish"}`), nasApi<Relationships>("social/relationships"),
      page === "publish" ? nasApi<{ media: Media[] }>("media") : Promise.resolve({ media: [] }),
    ]);
    if (generation.current !== current) return;
    if (!live) setPrefs(preferences);
    setItems(posts.posts); setNetworkStatus(posts.network_delivery); setRelations(relationships); setPending(uploads.media);
    const snapshot = JSON.stringify(relationships);
    if (relationshipSnapshot.current !== snapshot) { setConfirmed(false); relationshipSnapshot.current = snapshot; }
  }
  useEffect(() => {
    const current = ++generation.current;
    void refresh(current).catch(cause => { if (generation.current === current) setError(explain(cause)); });
    if (page === "relationships") void nasApi<{ blocked: string[] }>("social/blocks").then(result => { if (generation.current === current) setBlocked(result.blocked); }).catch(cause => { if (generation.current === current) setError(explain(cause)); });
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout> | undefined, loading = false, again = false;
    const update = async () => {
      if (generation.current !== current) return;
      if (loading) { again = true; return; }
      loading = true;
      try { await refresh(current, true); } catch (cause) { if (generation.current === current) setError(explain(cause)); }
      finally { loading = false; if (again && generation.current === current) { again = false; timer = setTimeout(() => void update(), 250); } }
    };
    void watchNasChanges(() => { if (timer) clearTimeout(timer); timer = setTimeout(() => void update(), 250); }, controller.signal).catch(cause => { if (generation.current === current) setError(explain(cause)); });
    return () => { generation.current++; controller.abort(); if (timer) clearTimeout(timer); };
  }, [owner, page]);
  async function run(task: () => Promise<void>, reload = true) {
    const current = generation.current; setBusy(true); setError(""); setMessage("");
    try { await task(); }
    catch (cause) { if (current === generation.current) setError(explain(cause)); }
    finally {
      if (reload && current === generation.current) {
        try { await refresh(current); } catch (cause) { if (current === generation.current) setError(explain(cause)); }
      }
      if (current === generation.current) setBusy(false);
    }
  }
  async function upload(files: File[]) {
    setConfirmed(false);
    if (media.length + files.length > (mode === "video" ? 1 : 9)) throw new Error("Choose up to 9 images or one video.");
    for (const file of files) {
      const allowed = mode === "video" ? ["video/mp4", "video/webm"] : ["image/jpeg", "image/png", "image/webp", "image/gif"];
      if (!allowed.includes(file.type) || file.size === 0 || file.size > (mode === "video" ? 64 : 10) * 1024 * 1024) throw new Error("Use images up to 10 MiB each, or one MP4/WebM video up to 64 MiB.");
    }
    for (const file of files) {
      setMessage(`Uploading ${file.name}…`);
      const uploaded = await nasUpload<Media>(file);
      const digest = `1220${Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", await file.arrayBuffer())), byte => byte.toString(16).padStart(2, "0")).join("")}`;
      if (uploaded.digest !== digest || uploaded.size !== file.size || uploaded.mime !== file.type) throw new Error("The NAS returned different media metadata. Nothing was published.");
      setMedia(previous => [...previous, uploaded]);
    }
    setMessage("Media saved privately. Sign and confirm the post to publish it.");
  }
  async function publish() {
    if (!confirmed) throw new Error("Confirm the content, price and recipients before publishing.");
    if (!title.trim() || new TextEncoder().encode(title).length > 256) throw new Error("Title must contain 1 to 256 UTF-8 bytes.");
    if ((!body.trim() && !media.length) || new TextEncoder().encode(body).length > 16384) throw new Error("Add text or media. Text must be within 16384 UTF-8 bytes.");
    if (mode !== "text" && !media.length) throw new Error("Upload media for the selected post format.");
    if (distribution !== "broadcast" && (!audience.length || audience.length > 32)) throw new Error("Choose 1 to 32 recipients. Narrow a larger audience using Specific people.");
    const draft: Draft = { title, body, distribution, price_wei: paid ? priceEthToWei(price) : "0", recipients: audience.map(addressBytes), ...(media.length ? { media } : {}) };
    setMessage("Confirm the publication signature in your wallet.");
    const result = await submitAction(owner, { action: "publish", post: draft });
    // The signed post now owns its uploads, even if the separate paid listing is interrupted.
    setTitle(""); setBody(""); setMedia([]); setConfirmed(false);
    if (result.status === "awaiting_listing") {
      setMessage("Post saved privately. Confirm the paid listing transaction in your wallet.");
      await listPaidPost(owner, result);
    }
    setMessage("Publication confirmed. Delivery continues in the background, including retries for offline recipients.");
  }
  async function read(post: PostSummary) { const value = await nasApi<PostDetail>(`posts/${post.id}`); setDetail(value); setSelected(post); }
  async function unlock(post: PostSummary, purchase: boolean) {
    const value = purchase && detail?.market ? await buyPost(owner, post, detail.market) : await nasApi<PostDetail>(`posts/${post.id}/unlock`, {});
    setDetail(value);
    if (value.intent) setItems(previous => previous.map(item => item.id === post.id ? { ...item, preview: value.intent!.action.post } : item));
  }
  return <div className="publishing">
    {error && <p className="error" role="alert">{error}</p>}{message && <p role="status">{message}</p>}
    <p className="muted">{networkStatus === "running" ? "Live NAS inbox · P2P delivery is running. Broadcasts reach connected subscribers." : networkStatus === "local_only" ? "Live NAS inbox · Remote delivery is waiting for P2P connectivity." : "Checking NAS delivery status…"}</p>
    {page === "publish" && <form className="content-panel post-composer" onSubmit={event => { event.preventDefault(); void run(publish); }} onChange={() => setConfirmed(false)}>
      <div className="composer-heading"><div><span className="eyebrow">CREATE A NOTE</span><h2>Share a moment</h2></div><span className="muted">Text, photos or video</span></div>
      <div className="format-tabs" role="group" aria-label="Post format">{([{ value: "text", title: "Text", Icon: FileText }, { value: "images", title: "Photos", Icon: ImagePlus }, { value: "video", title: "Video", Icon: Video }] as const).map(tab => <button type="button" aria-pressed={mode === tab.value} className={mode === tab.value ? "active" : ""} key={tab.value} disabled={busy} onClick={() => { if (media.length && mode !== tab.value) { setError("Remove existing uploads before changing the post format."); return; } setMode(tab.value); setConfirmed(false); }}><tab.Icon size={18} />{tab.title}</button>)}</div>
      <div className={`composer-layout ${mode === "text" ? "text-only" : ""}`}>
        {mode !== "text" && <section className="composer-media" aria-label="Post media">
          <label className="upload-area">{mode === "images" ? <ImagePlus size={30} /> : <Video size={30} />}<strong>{mode === "images" ? "Add your photos" : "Add a video"}</strong><span>{mode === "images" ? "Up to 9 images · 10 MiB each" : "One MP4 or WebM · up to 64 MiB"}</span><input aria-label="Choose media files" type="file" accept={mode === "images" ? "image/jpeg,image/png,image/webp,image/gif" : "video/mp4,video/webm"} multiple={mode === "images"} disabled={busy || media.length >= (mode === "images" ? 9 : 1)} onChange={event => { const files = Array.from(event.target.files ?? []); event.target.value = ""; void run(() => upload(files), false); }} /></label>
          <div className="upload-previews">{media.map((file, index) => <div className="upload-preview" key={file.id}><MediaView file={file} path={`media/${file.id}`} controls={file.mime.startsWith("video/")} /><span>{index === 0 ? "Cover" : index + 1}</span><button type="button" className="icon-button" aria-label={`Remove attachment ${index + 1}`} disabled={busy} onClick={() => void run(async () => { await nasApi(`media/${file.id}/discard`, {}); setMedia(previous => previous.filter(item => item.id !== file.id)); setConfirmed(false); }, false)}><X size={16} /></button></div>)}</div>
        </section>}
        <div className="composer-copy"><label>Title<input required maxLength={200} placeholder="Give your note a title" value={title} disabled={busy} onChange={event => setTitle(event.target.value)} /></label><label>Content {mode !== "text" && <small>(optional)</small>}<textarea required={mode === "text"} rows={8} maxLength={12000} placeholder="Tell your story…" value={body} disabled={busy} onChange={event => setBody(event.target.value)} /></label></div>
      </div>
      <div className="composer-settings"><label>Post type<select value={paid ? "paid" : "free"} disabled={busy} onChange={event => setPaid(event.target.value === "paid")}><option value="free">Free</option><option value="paid">Paid</option></select></label>{paid && <label>Price (Base Sepolia test ETH)<input type="text" required inputMode="decimal" value={price} disabled={busy} onChange={event => setPrice(event.target.value)} /></label>}
        <label>Distribution<select value={distribution} disabled={busy} onChange={event => setDistribution(event.target.value as Distribution)}>{Object.entries(scopes).map(([value, label]) => <option key={value} value={value}>{label}</option>)}</select></label>
      </div>
      {distribution === "directed" && <section className="recipient-picker"><h3>Select recipients</h3><div className="format-tabs"><button type="button" aria-pressed={peopleTab === "friends"} className={peopleTab === "friends" ? "active" : ""} onClick={() => setPeopleTab("friends")}>Friends ({friends.length})</button><button type="button" aria-pressed={peopleTab === "following"} className={peopleTab === "following" ? "active" : ""} onClick={() => setPeopleTab("following")}>Following ({relations.following.length})</button></div>
        <div className="people-options">{(peopleTab === "friends" ? friends : relations.following).map(value => <label className="checkbox-row" key={value}><input type="checkbox" checked={selectedPeople.includes(value)} disabled={busy} onChange={event => setSelectedPeople(previous => event.target.checked ? [...previous, value] : previous.filter(item => item !== value))} /><span className="key-value">{value}</span></label>)}{!(peopleTab === "friends" ? friends : relations.following).length && <p>No wallets in this list yet. You can enter an address below.</p>}</div>
        <label>Add wallet addresses<textarea rows={2} value={recipients} disabled={busy} onChange={event => setRecipients(event.target.value)} placeholder="0x… separated by commas or new lines" /></label>
      </section>}
      {distribution !== "broadcast" && <details className="audience-review" open><summary>{audience.length} recipient{audience.length === 1 ? "" : "s"} · Review before signing</summary>{!audience.length ? <p>No recipients available for this scope.</p> : audience.map(value => <p key={value} className="key-value">{value}</p>)}<p>Recipients are fixed when you sign. Future relationship changes do not change this post's audience. Maximum 32.</p></details>}
      <p className="composer-policy">{paid ? "Text and media stay locked until confirmed payment. The author NAS must stay available to deliver content. Listing and purchase transactions expose author, buyer and price. Base Sepolia test ETH only." : distribution === "broadcast" ? "This is a public post. Uploaded files stay private until you sign and confirm publication." : "This post is private to the reviewed recipients. Content is encrypted and media access uses the same audience permissions."}</p>
      <label className="checkbox-row"><input type="checkbox" checked={confirmed} disabled={busy} onChange={event => { event.stopPropagation(); setConfirmed(event.target.checked); }} />I confirm publication of this content, price and audience. A signature authorizes this publication.</label>
      <button className="button primary" disabled={busy || !confirmed} type="submit">{busy ? "Processing…" : "Sign and publish"}</button>
    </form>}
    {page === "publish" && pending.some(file => !media.some(selected => selected.id === file.id)) && <details className="content-panel"><summary>Unused uploads from earlier drafts</summary><p>Resume an upload or remove it to free storage. These files are visible only to you.</p>{pending.filter(file => !media.some(selected => selected.id === file.id)).map(file => <div className="relationship-row" key={file.id}><span>{file.mime} · {(file.size / 1024 / 1024).toFixed(1)} MiB</span><div className="binding-actions"><button type="button" className="button demo" disabled={busy} onClick={() => { const video = file.mime.startsWith("video/"); if (media.length && (video || media.some(item => item.mime.startsWith("video/")) || media.length >= 9)) { setError("Choose up to 9 images or one video."); return; } setMode(video ? "video" : "images"); setMedia(previous => [...previous, file]); setConfirmed(false); }}>Use in this post</button><button type="button" className="button demo" disabled={busy} onClick={() => void run(async () => { await nasApi(`media/${file.id}/discard`, {}); })}>Remove</button></div></div>)}</details>}
    {page === "feed" && <details className="content-panel"><summary>Receive posts from</summary>{(Object.keys(prefs) as (keyof Preferences)[]).map(name => <label className="checkbox-row" key={name}><input type="checkbox" checked={prefs[name]} disabled={busy} onChange={event => setPrefs({ ...prefs, [name]: event.target.checked })} />{{ broadcasts: "Network broadcasts", following: "People I follow", followers: "People who follow me", direct: "Posts addressed to me" }[name]}</label>)}<p>Selected sources are combined. Reception does not override audience or payment requirements.</p><button className="button demo" disabled={busy} onClick={() => void run(async () => { await nasApi("social/preferences", prefs); setMessage("Reception preferences saved."); })}>Save reception preferences</button></details>}
    {page === "relationships" && <section className="content-panel"><h2>Following and followers</h2><p>Follow changes are signed and synchronized with the target NAS when reachable.</p><label>Wallet address<input value={target} disabled={busy} onChange={event => setTarget(event.target.value)} /></label><button className="button primary" disabled={busy || !target} onClick={() => void run(async () => { await submitAction(owner, { action: "follow", target: addressBytes(target), enabled: true }); setTarget(""); setMessage("Follow confirmed."); })}>Sign and follow</button><h3>Following</h3>{!relations.following.length && <p>No followed wallets yet.</p>}{relations.following.map(value => <div className="relationship-row" key={value}><span>{value}{friends.includes(value) ? " · Friend" : ""}</span><button className="button demo" disabled={busy} onClick={() => void run(async () => { await submitAction(owner, { action: "follow", target: addressBytes(value), enabled: false }); setMessage("Unfollow confirmed."); })}>Unfollow</button></div>)}<h3>Followers</h3>{relations.followers.map(value => <p className="key-value" key={value}>{value}</p>)}<h3>Blocked wallets</h3><p>Blocked authors cannot passively deliver posts to your inbox.</p><button className="button demo" disabled={busy || !target} onClick={() => void run(async () => { addressBytes(target); await nasApi("social/blocks", { target, blocked: true }); setBlocked((await nasApi<{ blocked: string[] }>("social/blocks")).blocked); setTarget(""); setMessage("Wallet blocked."); })}>Block entered wallet</button>{blocked.map(value => <div className="relationship-row" key={value}><span>{value}</span><button className="button demo" disabled={busy} onClick={() => void run(async () => { await nasApi("social/blocks", { target: value, blocked: false }); setBlocked(previous => previous.filter(item => item !== value)); })}>Unblock</button></div>)}</section>}
    {page !== "relationships" && <><div className="binding-actions note-list-heading"><h2>{page === "publish" ? "My notes" : "Latest notes"}</h2><button className="button demo" disabled={busy} onClick={() => void run(async () => {})}>Refresh</button></div>{!items.length && <div className="notes-empty">No posts in this view yet.</div>}
      <div className="notes-grid">{items.map(post => { const preview = post.preview, cover = preview?.media?.[0]; return <article className="note-card" key={post.id}>
        <button className={`note-cover ${cover ? "" : "text-cover"}`} disabled={busy} onClick={() => void run(() => read(post), false)} aria-label={`Open ${preview?.title ?? post.title}`}>
          {cover ? <MediaView file={cover} path={`posts/${post.id}/media/${cover.id}`} /> : preview ? <div><FileText size={24} /><h3>{preview.title}</h3><p>{preview.body}</p></div> : <div><LockKeyhole size={30} /><span>Paid note</span></div>}
          {cover?.mime.startsWith("video/") && <span className="cover-indicator"><Video size={17} /></span>}{(preview?.media?.length ?? 0) > 1 && <span className="cover-indicator">1 / {preview!.media!.length}</span>}
        </button><div className="note-card-copy"><span className="note-price">{post.price_wei === "0" ? "Free" : `${formatEther(BigInt(post.price_wei))} test ETH`} · {scopes[post.distribution as Distribution] ?? post.distribution}</span><button className="note-title" disabled={busy} onClick={() => void run(() => read(post), false)}>{preview?.title ?? post.title}</button><div className="note-card-meta"><span>{shortAddress(post.author)}</span><time>{new Date(post.created_at * 1000).toLocaleDateString()}</time></div>
        {page === "publish" && post.published && <div className="delivery-progress"><p>{Object.entries(post.delivery ?? {}).map(([state, count]) => `${state.replaceAll("_", " ")}: ${count}`).join(" · ") || "Preparing delivery…"}</p><button className="button demo" disabled={busy} onClick={() => void run(async () => { const result = await nasApi<{ targets: { target: string; state: string; attempts: number; last_error: string | null }[] }>(`posts/${post.id}/delivery`); setProgress(previous => ({ ...previous, [post.id]: result.targets })); }, false)}>Delivery details</button>{progress[post.id]?.map(item => <p className="key-value" key={item.target}>{item.target === "broadcast" ? "Connected network" : shortAddress(item.target)} · {item.state} · attempts: {item.attempts}{item.last_error ? ` · ${item.last_error}` : ""}</p>)}</div>}
        {!post.published && <div className="pending-listing"><p>Saved privately · listing pending</p><button className="button demo" disabled={busy} onClick={() => void run(async () => { const status = await nasApi<PostStatus>(`posts/${post.id}/listing`); await listPaidPost(owner, status); setMessage("Paid post is now active."); })}>Complete paid listing</button></div>}
        </div></article>; })}</div>
    </>}
    {selected && detail && <Detail key={selected.id} post={selected} value={detail} busy={busy} error={error} close={() => { setSelected(null); setDetail(null); }} unlock={purchase => void run(() => unlock(selected, purchase), false)} />}
  </div>;
}
function explain(error: unknown) { return error instanceof Error ? error.message.slice(0, 500) : "The content request failed. Check your NAS and wallet."; }
