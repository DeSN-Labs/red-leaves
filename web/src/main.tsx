import React, { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import {
  ArrowDown,
  ArrowRight,
  Bookmark,
  Check,
  ChevronRight,
  Compass,
  Heart,
  Home,
  Leaf,
  LogOut,
  Menu,
  Play,
  Plus,
  Search,
  Server,
  ShieldCheck,
  Users,
  Wallet,
  X,
} from "lucide-react";
import { posts, photo, type Post } from "./data";
import { apiBaseUrl, checkNasSession, connectWallet, logoutWallet, sessionExpiresAt, shortAddress } from "./wallet";
const NasBinding = React.lazy(() => import("./NasBinding").then(module => ({ default: module.NasBinding })));
import "./styles.css";

function Brand() {
  return (
    <span className="brand">
      <span className="brand-icon">
        <Leaf size={23} strokeWidth={1.8} />
      </span>
      <strong>红叶</strong>
      <span className="brand-en">RED LEAVES</span>
    </span>
  );
}
type Session = { mode: "demo" | "wallet"; address?: string };
function App() {
  const [session, setSession] = useState<Session | null>(null);
  const [busy, setBusy] = useState(false);
  const [nasAvailable, setNasAvailable] = useState(true);
  const [checkingNas, setCheckingNas] = useState(false);
  const [error, setError] = useState("");
  const [page, setPage] = useState("关注动态");
  const [query, setQuery] = useState("");
  const [saved, setSaved] = useState<number[]>([]);
  const [liked, setLiked] = useState<number[]>([]);
  const [selected, setSelected] = useState<Post | null>(null);
  const [notice, setNotice] = useState("");
  const [mobileNav, setMobileNav] = useState(false);
  useEffect(() => {
    const provider = window.ethereum;
    const reset = () => {
      setSession(null);
      setSelected(null);
      setSaved([]);
      setLiked([]);
      setError("Wallet account or network changed. Please sign in again.");
      void logoutWallet().catch(() => setError("Signed out locally. The server session could not be revoked and will remain valid until it expires."));
    };
    provider?.on?.("accountsChanged", reset);
    provider?.on?.("chainChanged", reset);
    provider?.on?.("disconnect", reset);
    return () => {
      provider?.removeListener?.("accountsChanged", reset);
      provider?.removeListener?.("chainChanged", reset);
      provider?.removeListener?.("disconnect", reset);
    };
  }, [session?.mode]);
  useEffect(() => {
    if (session?.mode !== "wallet") return;
    const timer = setTimeout(() => {
      setSession(null);
      setSelected(null);
      setSaved([]);
      setLiked([]);
      setError("Your session has expired. Please sign in again.");
      void logoutWallet().catch(() => {});
    }, Math.max(0, sessionExpiresAt() * 1000 - Date.now()));
    return () => clearTimeout(timer);
  }, [session]);
  useEffect(() => {
    if (!notice) return;
    const timer = setTimeout(() => setNotice(""), 4000);
    return () => clearTimeout(timer);
  }, [notice]);
  useEffect(() => {
    if (!selected) return;
    const previous = document.activeElement as HTMLElement | null;
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    const close = (e: KeyboardEvent) => {
      if (e.key === "Escape") setSelected(null);
      if (e.key === "Tab") {
        const controls =
          document.querySelectorAll<HTMLButtonElement>(".post-modal button");
        const first = controls[0],
          last = controls[controls.length - 1];
        if (e.shiftKey && document.activeElement === first) {
          e.preventDefault();
          last?.focus();
        } else if (!e.shiftKey && document.activeElement === last) {
          e.preventDefault();
          first?.focus();
        }
      }
    };
    window.addEventListener("keydown", close);
    return () => {
      window.removeEventListener("keydown", close);
      document.body.style.overflow = previousOverflow;
      previous?.focus();
    };
  }, [selected]);
  const connect = async () => {
    setBusy(true);
    setError("");
    try {
      setSession({ mode: "wallet", address: await connectWallet() });
      setNasAvailable(true);
      setPage("我的 NAS");
    } catch (e) {
      const code = (e as { code?: number }).code;
      setError(
        code === 4001
          ? "You canceled the wallet request. You can try again at any time."
          : code === -32002
            ? "A wallet request is already pending. Open your wallet to complete it."
            : e instanceof Error
              ? e.message
              : "Connection failed. Check your wallet and try again.",
      );
    } finally {
      setBusy(false);
    }
  };
  const toggleSave = (id: number) => {
    setSaved((s) => (s.includes(id) ? s.filter((x) => x !== id) : [...s, id]));
    setNotice("Demo bookmarks updated for this session only. No content was downloaded to your NAS.");
  };
  const toggleLike = (id: number) =>
    setLiked((s) => (s.includes(id) ? s.filter((x) => x !== id) : [...s, id]));
  const navigate = (name: string) => {
    setPage(name);
    setQuery("");
    setMobileNav(false);
  };
  const checkNas = async (openPublishing = false) => {
    if (session?.mode !== "wallet") {
      setNotice("Sign in with your wallet to connect to the NAS. Demo mode has no NAS session.");
      return;
    }
    setCheckingNas(true);
    try {
      await checkNasSession();
      setNasAvailable(true);
      if (openPublishing) navigate("我的发布");
      else setNotice("NAS connection and wallet session verified.");
    } catch {
      setNasAvailable(false);
      setNotice("Unable to verify the NAS session. Check that the server is running and sign in again if your session has expired.");
    } finally {
      setCheckingNas(false);
    }
  };
  const nasConnected = session?.mode === "wallet" && nasAvailable;
  if (!session)
    return (
      <div className="login-page">
        <header className="login-header">
          <Brand />
          <span className="alpha">
            PRIVATE BY NATURE <span>·</span> 开发预览
          </span>
        </header>
        <main className="login-main">
          <section className="login-story">
            <div className="eyebrow">
              <span /> 一个属于你的社交网络
            </div>
            <h1>
              让连接自由生长，
              <br />
              让内容<span>真正属于你。</span>
            </h1>
            <p className="story-copy">
              不被算法定义，不让创作被抽成。
              <br />
              在红叶，用自己的身份，与在意的人分享生活。
            </p>
            <div className="story-image">
              <img
                src={photo("photo-1441974231531-c6227db76b6e", 1200)}
                alt="阳光穿过绿色森林"
              />
              <div className="image-caption">
                <Leaf size={19} />
                <span>根植于你的 NAS，连接更广阔的世界。</span>
              </div>
              <span className="image-tag">OWN YOUR CONNECTIONS</span>
            </div>
          </section>
          <section className="login-card">
            <span className="small-leaf">
              <Leaf size={30} />
            </span>
            <div className="eyebrow">YOUR WALLET. YOUR WORLD.</div>
            <h2>从你的钱包开始</h2>
            <p className="card-sub">无需注册账号，一个钱包地址就是你的身份。</p>
            <div className="benefits">
              <div>
                <ShieldCheck />
                <span>
                  <strong>身份由你掌握</strong>
                  <small>钱包私钥始终留在你的设备上</small>
                </span>
              </div>
              <div>
                <Server />
                <span>
                  <strong>内容存放在自己的 NAS</strong>
                  <small>自己的数据，自己保管</small>
                </span>
              </div>
              <div>
                <Users />
                <span>
                  <strong>关注你真正感兴趣的人</strong>
                  <small>没有算法推荐，只有你的选择</small>
                </span>
              </div>
            </div>
            <button
              className="button primary connect"
              onClick={connect}
              disabled={busy}
            >
              <Wallet size={19} />
              {busy ? "Confirm in your wallet…" : "Connect wallet"}
              <ArrowRight size={18} />
            </button>
            {error && (
              <p role="alert" className="error">
                {error}
              </p>
            )}
            <div className="or">
              <span />
              或先来看看
              <span />
            </div>
            <button
              className="button demo"
              disabled={busy}
              onClick={() => setSession({ mode: "demo" })}
            >
              进入演示空间 <ArrowUpRight />
            </button>
            <p className="login-note">
              Sign a one-time message to log in. No transfers or private keys are requested.
              <br />
              A running NAS node is required. Only Base Sepolia EOA wallets are supported.
            </p>
            <div className="network-label">
              <span /> Base Sepolia · 开发网络
            </div>
          </section>
        </main>
        <footer className="login-footer">
          <span>© 2026 红叶 Red Leaves</span>
          <span>自由连接 · 自主存储 · 无平台抽成</span>
          <span>
            让每一份创作，回到创作者手中 <ArrowDown size={13} />
          </span>
        </footer>
      </div>
    );
  const visible = posts.filter(
    (p) =>
      (page !== "我的收藏" || saved.includes(p.id)) &&
      ((page !== "山间来信" && page !== "阿禾的日常" && page !== "字节花园") ||
        p.author === page) &&
      `${p.title}${p.author}${p.description}`.includes(query),
  );
  const feedPage = !["我的 NAS", "我的发布"].includes(page);
  return (
    <div className="app-shell">
      <header className="app-header">
        <div className="header-brand">
          <button
            className="icon-button mobile-menu"
            aria-label="Open navigation"
            onClick={() => setMobileNav(!mobileNav)}
          >
            <Menu />
          </button>
          <Brand />
        </div>
        <form className="search" onSubmit={(e) => e.preventDefault()}>
          <Search size={19} />
          <input
            aria-label="Search demo content"
            placeholder="Search content or creators…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
        </form>
        <div className="header-actions">
          <span className="preview-badge">演示内容</span>
          <button
            className="button publish"
            disabled={checkingNas}
            onClick={() => void checkNas(true)}
          >
            <Plus size={17} />
            发布内容
          </button>
          <button
            className="user-avatar"
            title="Current identity"
            onClick={() =>
              setNotice(
                session.address
                  ? `Wallet ${session.address} authenticated with NAS ${apiBaseUrl}. On-chain binding has not been checked.`
                  : "You are using a demo identity with no linked wallet.",
              )
            }
          >
            我
          </button>
        </div>
      </header>
      {mobileNav && (
        <button
          className="nav-backdrop"
          aria-label="Close navigation"
          onClick={() => setMobileNav(false)}
        />
      )}
      <aside className={`sidebar ${mobileNav ? "open" : ""}`}>
        <nav aria-label="Main navigation">
          <p className="nav-label">我的空间</p>
          {[
            { name: "关注动态", Icon: Home },
            { name: "我的关注", Icon: Users },
            { name: "我的收藏", Icon: Bookmark },
            { name: "我的发布", Icon: Play },
          ].map(({ name, Icon }) => (
            <button
              key={name}
              className={`nav-item ${page === name ? "active" : ""}`}
              onClick={() => navigate(name)}
            >
              <Icon size={19} />
              {name}
              {name === "我的收藏" && saved.length > 0 && (
                <span className="count">{saved.length}</span>
              )}
            </button>
          ))}
          <div className="nav-divider" />
          <p className="nav-label">
            关注的创作者 <span>03</span>
          </p>
          {[
            { name: "山间来信", letter: "山", color: "#758c73" },
            { name: "阿禾的日常", letter: "禾", color: "#bc8e69" },
            { name: "字节花园", letter: "字", color: "#71899b" },
          ].map((a) => (
            <button
              className={`nav-item ${page === a.name ? "active" : ""}`}
              key={a.name}
              onClick={() => navigate(a.name)}
            >
              <span className="avatar small" style={{ background: a.color }}>
                {a.letter}
              </span>
              {a.name}
            </button>
          ))}
          <div className="nav-divider" />
          <button
            className={`nav-item ${page === "我的 NAS" ? "active" : ""}`}
            onClick={() => navigate("我的 NAS")}
          >
            <Server size={19} />
            我的 NAS
          </button>
        </nav>
        <div className="sidebar-bottom">
          <div className="nas-mini">
            <span className="nas-symbol">
              <Server size={20} />
            </span>
            <div>
              <strong>你的数字家园</strong>
              <small>
                <span className={`status-dot ${nasConnected ? "" : "disconnected"}`} />
                {session.mode === "demo" ? "Demo mode · No NAS session" : nasConnected ? "NAS connected" : "NAS unavailable"}
              </small>
            </div>
            <ChevronRight size={15} />
          </div>
          <div className="wallet-mini">
            <Wallet size={15} />
            <span>
              {session.address ? shortAddress(session.address) : "演示身份"}
            </span>
            <button
              className="icon-button"
              aria-label="Sign out"
              onClick={() => {
                void logoutWallet().catch(() => setError("Signed out locally. The server session could not be revoked and will remain valid until it expires."));
                setSession(null);
                setSelected(null);
                setSaved([]);
                setLiked([]);
                setPage("关注动态");
                setQuery("");
              }}
            >
              <LogOut size={16} />
            </button>
          </div>
          <p className="sidebar-foot">
            内容属于你，连接也属于你。
            <br />
            RED LEAVES · 开发预览
          </p>
        </div>
      </aside>
      <main className="content">
        <div className="breadcrumb">
          我的空间 <ChevronRight size={13} />
          <span>{page}</span>
        </div>
        {session.mode === "wallet" && session.address && <React.Suspense fallback={<p role="status">Checking profile registration…</p>}><NasBinding key={session.address} owner={session.address} /></React.Suspense>}
        <div className="page-heading">
          <div>
            <h1>{page === "关注动态" ? "每一份分享，都值得停留。" : page}</h1>
            <p>
              {page === "关注动态"
                ? "来自你关注的人，按时间自然发生。"
                : page === "我的收藏"
                  ? "Save moments to revisit. Bookmarks are kept for this session only."
                  : page === "我的 NAS"
                    ? "连接自己的设备，让数据回到自己的手中。"
                    : "你的选择，构成你的内容世界。"}
            </p>
          </div>
          <span className="date-label">RED LEAVES / YOUR SPACE</span>
        </div>
        {page === "我的关注" ? (
          <section className="creator-list">
            {["山间来信", "阿禾的日常", "字节花园"].map((name) => (
              <button key={name} onClick={() => navigate(name)}>
                <span
                  className="avatar"
                  style={{
                    background: posts.find((p) => p.author === name)?.color,
                  }}
                >
                  {name[0]}
                </span>
                <strong>{name}</strong>
                <span>查看演示动态</span>
                <ChevronRight size={18} />
              </button>
            ))}
          </section>
        ) : page === "我的 NAS" ? (
          <section className="empty-state">
            <Server size={42} />
            <h2>{nasConnected ? "NAS connected" : session.mode === "demo" ? "Demo mode" : "NAS unavailable"}</h2>
            <p>{session.mode === "wallet"
              ? `NAS API: ${apiBaseUrl}. ${nasConnected ? "Your wallet has authenticated with this NAS." : "Check the server connection and verify your session again."}`
              : "Sign in with your wallet to use the configured NAS. Demo mode does not connect to the server."}</p>
            {session.mode === "wallet" && (
              <button className="button demo" disabled={checkingNas} onClick={() => void checkNas()}>
                {checkingNas ? "Checking NAS…" : "Check NAS connection"}
              </button>
            )}

          </section>
        ) : page === "我的发布" ? (
          <section className="empty-state">
            <Leaf size={42} />
            <h2>{nasConnected ? "NAS connected · Publishing pending" : "Publishing is not available yet"}</h2>
            <p>{nasConnected
              ? "You are authenticated with your NAS. The content publishing and file upload APIs are not implemented yet."
              : "Sign in with your wallet and verify the NAS connection. Content publishing and file uploads are not implemented yet."}</p>
            <button
              className="button demo"
              onClick={() => navigate("关注动态")}
            >
              浏览关注动态 <ArrowRight size={16} />
            </button>
          </section>
        ) : (
          <>
            {page === "关注动态" && (
              <section className="welcome-banner">
                <div>
                  <span className="eyebrow">LESS NOISE. MORE CONNECTION.</span>
                  <h2>
                    这里的每一片叶子，
                    <br />
                    都来自你的选择。
                  </h2>
                  <p>没有为你推荐，只有你主动关注的世界。</p>
                </div>
                <div className="banner-art" aria-hidden="true">
                  <span className="orbit orbit-one" />
                  <span className="orbit orbit-two" />
                  <Leaf className="leaf-one" />
                  <Leaf className="leaf-two" />
                  <Leaf className="leaf-three" />
                  <span className="art-caption">GROW AT YOUR OWN PACE</span>
                </div>
                <span className="banner-number">01 / 自由生长</span>
              </section>
            )}
            {feedPage && (
              <>
                <div className="feed-toolbar">
                  <span>{page === "关注动态" ? "关注内容" : "时间线"}</span>
                  <span className="sort-label">
                    最新发布 <ArrowDown size={14} />
                  </span>
                </div>
                <div className="section-label">
                  <span>
                    {query
                      ? `搜索「${query}」`
                      : page === "我的收藏"
                        ? "收藏的内容"
                        : "最近的分享"}
                  </span>
                  <small>{visible.length} 份演示内容</small>
                </div>
                <section
                  className={page === "关注动态" ? "post-grid" : "post-timeline"}
                  aria-label={page === "关注动态" ? "关注内容卡片" : "内容时间线"}
                >
                  {visible.map((post) => (
                    <article className="post-card" key={post.id}>
                      <button
                        className="thumbnail"
                        onClick={() => setSelected(post)}
                        aria-label={`Read: ${post.title}`}
                      >
                        <img
                          src={photo(post.image)}
                          alt={post.title}
                          loading="lazy"
                        />
                        {post.duration && (
                          <span className="duration">{post.duration}</span>
                        )}
                        <span className="play-hover">
                          <Play size={24} fill="currentColor" />
                        </span>
                      </button>
                      <div className="post-info">
                        <button
                          className="avatar"
                          style={{ background: post.color }}
                          onClick={() => navigate(post.author)}
                          aria-label={`View ${post.author}`}
                        >
                          {post.avatar}
                        </button>
                        <div className="post-copy">
                          <button
                            className="post-title"
                            onClick={() => setSelected(post)}
                          >
                            {post.title}
                          </button>
                          <p>
                            {post.author}
                            <span>·</span>
                            {post.time}
                          </p>
                          <div className="post-meta">
                            <span>
                              <Heart size={13} />
                              {post.likes + (liked.includes(post.id) ? 1 : 0)}
                            </span>
                            <span>公开分享</span>
                          </div>
                        </div>
                        <button
                          className={`icon-button save ${saved.includes(post.id) ? "is-saved" : ""}`}
                          aria-label={
                            saved.includes(post.id)
                              ? "Remove demo bookmark"
                              : "Bookmark demo content"
                          }
                          onClick={() => toggleSave(post.id)}
                        >
                          <Bookmark
                            size={17}
                            fill={
                              saved.includes(post.id) ? "currentColor" : "none"
                            }
                          />
                        </button>
                      </div>
                    </article>
                  ))}
                </section>
                {visible.length === 0 && (
                  <section className="empty-state">
                    <Compass size={36} />
                    <h2>
                      {page === "我的收藏"
                        ? "No bookmarks yet"
                        : "No matching content found"}
                    </h2>
                    <p>
                      {page === "我的收藏"
                        ? "Select the bookmark on a content card to save demo content."
                        : "Try a different search term."}
                    </p>
                  </section>
                )}
              </>
            )}
          </>
        )}
        <footer className="content-footer">
          <Leaf size={14} />
          <span>好的连接，自然发生。</span>
          <span>演示数据 · 无算法推荐</span>
        </footer>
      </main>
      {notice && (
        <div className="toast" role="status">
          <Check size={17} />
          {notice}
        </div>
      )}
      {selected && (
        <div className="modal-backdrop" onClick={() => setSelected(null)}>
          <section
            className="post-modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="post-dialog-title"
            onClick={(e) => e.stopPropagation()}
          >
            <button
              autoFocus
              className="modal-close icon-button"
              aria-label="Close content"
              onClick={() => setSelected(null)}
            >
              <X />
            </button>
            <img src={photo(selected.image, 1200)} alt={selected.title} />
            <div className="modal-body">
              <span className="eyebrow">演示内容</span>
              <h2 id="post-dialog-title">{selected.title}</h2>
              <p className="modal-author">
                {selected.author} · {selected.time}
              </p>
              <p>{selected.description}</p>
              <p className="muted">
                This is a content display prototype. Video playback and NAS file transfers are not available yet.
              </p>
              <div className="modal-actions">
                <button
                  className={`button demo ${liked.includes(selected.id) ? "liked" : ""}`}
                  onClick={() => toggleLike(selected.id)}
                >
                  <Heart
                    size={17}
                    fill={liked.includes(selected.id) ? "currentColor" : "none"}
                  />
                  {liked.includes(selected.id) ? "Liked" : "Like"} ·{" "}
                  {selected.likes + (liked.includes(selected.id) ? 1 : 0)}
                </button>
                <button
                  className="button demo"
                  onClick={() => toggleSave(selected.id)}
                >
                  <Bookmark size={17} />
                  {saved.includes(selected.id) ? "Bookmarked" : "Bookmark"}
                </button>
              </div>
            </div>
          </section>
        </div>
      )}
    </div>
  );
}
function ArrowUpRight() {
  return <ArrowRight size={17} style={{ transform: "rotate(-35deg)" }} />;
}

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
