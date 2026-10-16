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
import { posts, photo, type Category, type Post } from "./data";
import { connectWallet, shortAddress } from "./wallet";
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
  const [error, setError] = useState("");
  const [page, setPage] = useState("关注动态");
  const [category, setCategory] = useState<Category>("全部");
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
      setError("钱包账户或网络已变更，请重新连接。");
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
    } catch (e) {
      const code = (e as { code?: number }).code;
      setError(
        code === 4001
          ? "你取消了钱包连接，可以随时重试。"
          : code === -32002
            ? "钱包中已有待处理请求，请打开钱包完成连接。"
            : e instanceof Error
              ? e.message
              : "连接失败，请检查钱包后重试。",
      );
    } finally {
      setBusy(false);
    }
  };
  const toggleSave = (id: number) => {
    setSaved((s) => (s.includes(id) ? s.filter((x) => x !== id) : [...s, id]));
    setNotice("仅更新本次演示收藏，尚未下载到 NAS。");
  };
  const toggleLike = (id: number) =>
    setLiked((s) => (s.includes(id) ? s.filter((x) => x !== id) : [...s, id]));
  const navigate = (name: string) => {
    setPage(name);
    setQuery("");
    setCategory("全部");
    setMobileNav(false);
  };
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
              {busy ? "请在钱包中确认…" : "连接钱包"}
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
              onClick={() => setSession({ mode: "demo" })}
            >
              进入演示空间 <ArrowUpRight />
            </button>
            <p className="login-note">
              连接仅请求公开地址，不请求转账或私钥。
              <br />
              此版本尚未接入 NAS 登录认证。
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
      (category === "全部" || p.category === category) &&
      `${p.title}${p.author}${p.description}`.includes(query),
  );
  const feedPage = !["我的 NAS", "我的发布"].includes(page);
  return (
    <div className="app-shell">
      <header className="app-header">
        <div className="header-brand">
          <button
            className="icon-button mobile-menu"
            aria-label="打开导航"
            onClick={() => setMobileNav(!mobileNav)}
          >
            <Menu />
          </button>
          <Brand />
        </div>
        <form className="search" onSubmit={(e) => e.preventDefault()}>
          <Search size={19} />
          <input
            aria-label="搜索演示内容"
            placeholder="搜索内容、创作者…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
        </form>
        <div className="header-actions">
          <span className="preview-badge">演示内容</span>
          <button
            className="button publish"
            onClick={() =>
              setNotice("发布功能待接入 NAS，当前不会签名或发送内容。")
            }
          >
            <Plus size={17} />
            发布内容
          </button>
          <button
            className="user-avatar"
            title="当前身份"
            onClick={() =>
              setNotice(
                session.address
                  ? `已连接 ${session.address}；尚未完成 NAS 认证。`
                  : "当前为演示身份，不关联真实钱包。",
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
          aria-label="关闭导航"
          onClick={() => setMobileNav(false)}
        />
      )}
      <aside className={`sidebar ${mobileNav ? "open" : ""}`}>
        <nav aria-label="主导航">
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
                <span className="status-dot" />
                NAS 尚未连接
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
              aria-label="退出登录"
              onClick={() => {
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
        <div className="page-heading">
          <div>
            <h1>{page === "关注动态" ? "每一份分享，都值得停留。" : page}</h1>
            <p>
              {page === "关注动态"
                ? "来自你关注的人，按时间自然发生。"
                : page === "我的收藏"
                  ? "留住想要再次回看的片刻。本次收藏仅在当前页面有效。"
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
            <h2>还没有连接常驻 NAS</h2>
            <p>
              钱包连接不代表已完成设备绑定。注册合约和节点接口接入后，你可以在这里管理自己的数字家园。
            </p>
            <span className="network-label">
              Base Sepolia · 尚未查询链上绑定
            </span>
          </section>
        ) : page === "我的发布" ? (
          <section className="empty-state">
            <Leaf size={42} />
            <h2>你的故事，从这里开始</h2>
            <p>
              发布功能尚未接入。未来内容将先保存在自己的 NAS，再向网络传播。
            </p>
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
                  <div className="tabs" aria-label="内容类别">
                    {(
                      ["全部", "影像", "生活", "技术", "自然"] as Category[]
                    ).map((c) => (
                      <button
                        key={c}
                        className={category === c ? "selected" : ""}
                        onClick={() => setCategory(c)}
                      >
                        {c}
                      </button>
                    ))}
                  </div>
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
                <section className="post-grid">
                  {visible.map((post) => (
                    <article className="post-card" key={post.id}>
                      <button
                        className="thumbnail"
                        onClick={() => setSelected(post)}
                        aria-label={`阅读：${post.title}`}
                      >
                        <img
                          src={photo(post.image)}
                          alt={post.title}
                          loading="lazy"
                        />
                        <span className="category-badge">{post.category}</span>
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
                          aria-label={`查看${post.author}`}
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
                              ? "取消演示收藏"
                              : "收藏演示内容"
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
                        ? "还没有收藏内容"
                        : "没有找到相关内容"}
                    </h2>
                    <p>
                      {page === "我的收藏"
                        ? "点击内容卡片的书签，收藏喜欢的演示内容。"
                        : "试试其他关键词或内容类别。"}
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
              aria-label="关闭内容"
              onClick={() => setSelected(null)}
            >
              <X />
            </button>
            <img src={photo(selected.image, 1200)} alt={selected.title} />
            <div className="modal-body">
              <span className="eyebrow">{selected.category} · 演示内容</span>
              <h2 id="post-dialog-title">{selected.title}</h2>
              <p className="modal-author">
                {selected.author} · {selected.time}
              </p>
              <p>{selected.description}</p>
              <p className="muted">
                这是内容展示原型，尚无实际视频播放或 NAS 文件传输。
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
                  {liked.includes(selected.id) ? "已点赞" : "点赞"} ·{" "}
                  {selected.likes + (liked.includes(selected.id) ? 1 : 0)}
                </button>
                <button
                  className="button demo"
                  onClick={() => toggleSave(selected.id)}
                >
                  <Bookmark size={17} />
                  {saved.includes(selected.id) ? "已收藏" : "收藏"}
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
