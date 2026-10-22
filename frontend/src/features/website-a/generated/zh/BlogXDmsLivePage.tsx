// Generated from approved A, SHA-256 5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33.
// Run: node scripts/website-a/generate.mjs <baseline.html>
import { VisualLink, RevealFrame } from "../../visuals/elements";
import { useWebsite } from "../../website-context";
export default function BlogXDmsLivePage() {
  const s = useWebsite();
  void s;
  return (
    <>
      {"\n"}
      <section
        className="s glowbg"
        style={{ paddingBottom: "40px", position: "relative" }}
      >
        {"\n  "}
        <div className="wrap article">
          <div className="crumbs">
            <a href={s.siteHref("/blog")}>{"博客"}</a>
            <span>{"/"}</span>
            <span>{"产品更新"}</span>
          </div>
          <h1 className="h1" style={{ fontSize: "44px" }}>
            {"产品更新：X（推特）私信已上线"}
          </h1>
          <p className="lead" style={{ marginTop: "16px" }}>
            {
              "你的助手现在可以回复 X 私信，与 Telegram 一样使用限权密钥和审批。"
            }
          </p>
          <div className="author" style={{ marginTop: "22px" }}>
            <span className="a"></span>
            <div>
              <b>{"NyxID 团队"}</b>
              <div className="small">{"2026年8月28日 · 2 分钟阅读"}</div>
            </div>
          </div>
        </div>
        {"\n"}
      </section>
      {"\n"}
      <div className="wrap" style={{ maxWidth: "900px" }}>
        <RevealFrame
          className="cover"
          style={{
            background: "linear-gradient(135deg,#18181B,#3F3F46)",
            borderRadius: "20px",
            aspectRatio: "21/9",
          }}
        >
          <svg className="i" viewBox="0 0 24 24">
            <path d="m3 11 18-5v12L3 14v-3z"></path>
            <path d="M11.6 16.8a3 3 0 1 1-5.8-1.6"></path>
          </svg>
        </RevealFrame>
      </div>
      {"\n"}
      <section className="s" style={{ paddingTop: "40px", border: "none" }}>
        <div className="wrap article">
          <p>
            {
              "连接你的 X 账号并选择一个助手。它会回复私信、筛选并记录线索，除非你允许，否则不能公开发帖。"
            }
          </p>
          <ul>
            <li>{"通过 X 托管登录，无需开发者配置。"}</li>
            <li>{"从最新消息开始，不会重放旧消息。"}</li>
            <li>{"同样的限权密钥、审批和操作记录。"}</li>
          </ul>
          <p>{"接下来：更多聊天应用。告诉我们你需要哪一个。"}</p>
          {"\n  "}
          <div
            className="card"
            style={{
              marginTop: "44px",
              display: "flex",
              justifyContent: "space-between",
              alignItems: "center",
              gap: "16px",
              flexWrap: "wrap",
            }}
          >
            <div>
              <b>{"准备试试吗？"}</b>
              <div className="small">
                {"今天就让 NyxBot 设置你的第一项工作。"}
              </div>
            </div>
            <div style={{ display: "flex", gap: "8px", flexWrap: "wrap" }}>
              <a className="btn" href={s.siteHref("/solutions")}>
                {"浏览方案"}
              </a>
              <a
                className="btn"
                href="https://discord.gg/QMvcs8UQBW"
                target="_blank"
                rel="noopener noreferrer"
              >
                <svg
                  className="dc"
                  width="16"
                  height="16"
                  viewBox="0 0 24 24"
                  aria-hidden="true"
                >
                  <path d="M20.317 4.37a19.791 19.791 0 0 0-4.885-1.515.074.074 0 0 0-.079.037c-.21.375-.444.864-.608 1.25a18.27 18.27 0 0 0-5.487 0 12.64 12.64 0 0 0-.617-1.25.077.077 0 0 0-.079-.037A19.736 19.736 0 0 0 3.677 4.37a.07.07 0 0 0-.032.027C.533 9.046-.32 13.58.099 18.057a.082.082 0 0 0 .031.057 19.9 19.9 0 0 0 5.993 3.03.078.078 0 0 0 .084-.028c.462-.63.874-1.295 1.226-1.994a.076.076 0 0 0-.041-.106 13.107 13.107 0 0 1-1.872-.892.077.077 0 0 1-.008-.128 10.2 10.2 0 0 0 .372-.292.074.074 0 0 1 .077-.01c3.928 1.793 8.18 1.793 12.062 0a.074.074 0 0 1 .078.01c.12.098.246.198.373.292a.077.077 0 0 1-.006.127 12.299 12.299 0 0 1-1.873.892.077.077 0 0 0-.041.107c.36.698.772 1.362 1.225 1.993a.076.076 0 0 0 .084.028 19.839 19.839 0 0 0 6.002-3.03.077.077 0 0 0 .032-.054c.5-5.177-.838-9.674-3.549-13.66a.061.061 0 0 0-.031-.03zM8.02 15.33c-1.183 0-2.157-1.085-2.157-2.419 0-1.333.956-2.419 2.157-2.419 1.21 0 2.176 1.095 2.157 2.42 0 1.333-.956 2.418-2.157 2.418zm7.975 0c-1.183 0-2.157-1.085-2.157-2.419 0-1.333.956-2.419 2.157-2.419 1.21 0 2.176 1.095 2.157 2.42 0 1.333-.947 2.418-2.157 2.418z"></path>
                </svg>
                {" 在 Discord 讨论"}
              </a>
            </div>
          </div>
          {"\n"}
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap">
          <div className="sechead">
            <h2 className="h2">{"继续阅读"}</h2>
            <a className="btn" href={s.siteHref("/blog")}>
              {"全部文章 "}
              <svg className="i" viewBox="0 0 24 24">
                <path d="M5 12h14M12 5l7 7-7 7"></path>
              </svg>
            </a>
          </div>
          <div className="grid g3">
            <VisualLink
              className="card post-card"
              href={s.siteHref("/blog/what-is-nyxid")}
              effect="card"
            >
              <RevealFrame
                className="cover"
                style={{
                  background: "linear-gradient(135deg,#A672FB,#5E00F5)",
                }}
              >
                <svg className="i" viewBox="0 0 24 24">
                  <path d="m12 2 10 5-10 5L2 7l10-5z"></path>
                  <path d="m2 17 10 5 10-5M2 12l10 5 10-5"></path>
                </svg>
              </RevealFrame>
              <div className="body">
                <span className="badge b-v" style={{ alignSelf: "flex-start" }}>
                  {"指南"}
                </span>
                <h3>{"什么是“智能体网关”？写给企业主的通俗指南"}</h3>
                <p>
                  {"用 AI 不需要推倒重来。看看网关如何连接 AI 与你现有的工具。"}
                </p>
                <div className="meta">
                  <span>{"2026年10月10日"}</span>
                  <span>{"·"}</span>
                  <span>{"6 分钟阅读"}</span>
                </div>
              </div>
            </VisualLink>
            <VisualLink
              className="card post-card"
              href={s.siteHref("/blog/luna-bistro")}
              effect="card"
            >
              <RevealFrame
                className="cover"
                style={{
                  background: "linear-gradient(135deg,#5A2AF1,#B59FF9)",
                }}
              >
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M3 2v7c0 1.1.9 2 2 2h4a2 2 0 0 0 2-2V2"></path>
                  <path d="M7 2v20"></path>
                  <path d="M21 15V2a5 5 0 0 0-5 5v6c0 1.1.9 2 2 2h3Zm0 0v7"></path>
                </svg>
              </RevealFrame>
              <div className="body">
                <span className="badge b-v" style={{ alignSelf: "flex-start" }}>
                  {"客户故事"}
                </span>
                <h3>{"一家 40 座小酒馆如何不再错过深夜订位"}</h3>
                <p>
                  {
                    "示例故事：打烊后的消息过去要等到第二天早上，现在几秒钟就能订好。"
                  }
                </p>
                <div className="meta">
                  <span>{"2026年10月3日"}</span>
                  <span>{"·"}</span>
                  <span>{"4 分钟阅读"}</span>
                </div>
              </div>
            </VisualLink>
            <VisualLink
              className="card post-card"
              href={s.siteHref("/blog/never-share-passwords")}
              effect="card"
            >
              <RevealFrame
                className="cover"
                style={{
                  background: "linear-gradient(135deg,#1E0A68,#5A2AF1)",
                }}
              >
                <svg className="i" viewBox="0 0 24 24">
                  <rect x="3" y="11" width="18" height="11" rx="2"></rect>
                  <path d="M7 11V7a5 5 0 0 1 10 0v4"></path>
                </svg>
              </RevealFrame>
              <div className="body">
                <span className="badge b-v" style={{ alignSelf: "flex-start" }}>
                  {"安全"}
                </span>
                <h3>{"为什么你的 AI 永远不该看到你的密码"}</h3>
                <p>
                  {
                    "把密码粘贴给 AI 工具，就像把钥匙藏在门垫下。这里有更安全的做法。"
                  }
                </p>
                <div className="meta">
                  <span>{"2026年9月26日"}</span>
                  <span>{"·"}</span>
                  <span>{"5 分钟阅读"}</span>
                </div>
              </div>
            </VisualLink>
          </div>
        </div>
      </section>
    </>
  );
}
