// Generated from approved A, SHA-256 5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33.
// Run: node scripts/website-a/generate.mjs <baseline.html>
import { VisualLink, RevealFrame } from "../../visuals/elements";
import { useWebsite } from "../../website-context";
export default function BlogPage() {
  const s = useWebsite();
  void s;
  return (
    <>
      {"\n"}
      <section
        className="hero glowbg"
        style={{ paddingBottom: "40px", position: "relative" }}
      >
        {"\n  "}
        <div className="wrap">
          <div className="over v">{"Blog"}</div>
          <h1
            className="h1"
            style={{ marginTop: "14px", fontSize: "var(--fs-h1)" }}
          >
            {"Guides, stories & updates"}
          </h1>
          <p className="lead">
            {"Practical ideas for putting AI to work, safely."}
          </p>
        </div>
        {"\n"}
      </section>
      {"\n"}
      <section className="s" style={{ paddingTop: "20px" }}>
        <div className="wrap">
          {"\n  "}
          <VisualLink
            className="card featpost clk"
            href={s.siteHref("/blog/what-is-nyxid")}
            effect="card"
          >
            <RevealFrame
              className="cover"
              style={{ background: "linear-gradient(135deg,#A672FB,#5E00F5)" }}
            >
              <svg className="i" viewBox="0 0 24 24">
                <path d="m12 2 10 5-10 5L2 7l10-5z"></path>
                <path d="m2 17 10 5 10-5M2 12l10 5 10-5"></path>
              </svg>
            </RevealFrame>
            <div className="body">
              <span className="badge b-v" style={{ alignSelf: "flex-start" }}>
                {"Featured · Guides"}
              </span>
              <h2 className="h3" style={{ fontSize: "28px" }}>
                {
                  "What is an “agent gateway”? A plain-English guide for business owners"
                }
              </h2>
              <p className="sub">
                {
                  "You don’t need to rebuild anything to use AI. Here’s how a gateway sits between AI and the tools you already have."
                }
              </p>
              <div className="small">
                {"Oct 10, 2026 · 6 min read · NyxID team"}
              </div>
              <span className="more">
                {"Read article "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </span>
            </div>
          </VisualLink>
          {"\n  "}
          <div
            className="pills"
            id="blogF"
            style={{ margin: "40px 0 24px", justifyContent: "center" }}
          >
            <button
              className={s.filterClass("blogF", "all")}
              data-v="all"
              type="button"
              onClick={() => s.setFilter("blogF", "all")}
              aria-pressed={s.isFilterSelected("blogF", "all")}
            >
              {"All"}
            </button>
            <button
              className={s.filterClass("blogF", "guides")}
              data-v="guides"
              type="button"
              onClick={() => s.setFilter("blogF", "guides")}
              aria-pressed={s.isFilterSelected("blogF", "guides")}
            >
              {"Guides"}
            </button>
            <button
              className={s.filterClass("blogF", "stories")}
              data-v="stories"
              type="button"
              onClick={() => s.setFilter("blogF", "stories")}
              aria-pressed={s.isFilterSelected("blogF", "stories")}
            >
              {"Customer stories"}
            </button>
            <button
              className={s.filterClass("blogF", "security")}
              data-v="security"
              type="button"
              onClick={() => s.setFilter("blogF", "security")}
              aria-pressed={s.isFilterSelected("blogF", "security")}
            >
              {"Security"}
            </button>
            <button
              className={s.filterClass("blogF", "dev")}
              data-v="dev"
              type="button"
              onClick={() => s.setFilter("blogF", "dev")}
              aria-pressed={s.isFilterSelected("blogF", "dev")}
            >
              {"Developers"}
            </button>
            <button
              className={s.filterClass("blogF", "updates")}
              data-v="updates"
              type="button"
              onClick={() => s.setFilter("blogF", "updates")}
              aria-pressed={s.isFilterSelected("blogF", "updates")}
            >
              {"Product updates"}
            </button>
          </div>
          {"\n  "}
          <div className="grid g3" id="blogGrid">
            <VisualLink
              className="card post-card"
              href={s.siteHref("/blog/luna-bistro")}
              data-category="stories"
              hidden={!s.postVisible("stories")}
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
                  {"Customer stories"}
                </span>
                <h3>
                  {"How a 40-seat bistro stopped missing late-night bookings"}
                </h3>
                <p>
                  {
                    "An illustrative story: after-hours messages used to wait until morning. Now they’re booked in seconds."
                  }
                </p>
                <div className="meta">
                  <span>{"Oct 3, 2026"}</span>
                  <span>{"·"}</span>
                  <span>{"4 min read"}</span>
                </div>
              </div>
            </VisualLink>
            <VisualLink
              className="card post-card"
              href={s.siteHref("/blog/never-share-passwords")}
              data-category="security"
              hidden={!s.postVisible("security")}
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
                  {"Security"}
                </span>
                <h3>{"Why your AI should never see your passwords"}</h3>
                <p>
                  {
                    "Pasting a password into an AI tool is like leaving your keys under the mat. Here’s the safer way."
                  }
                </p>
                <div className="meta">
                  <span>{"Sep 26, 2026"}</span>
                  <span>{"·"}</span>
                  <span>{"5 min read"}</span>
                </div>
              </div>
            </VisualLink>
            <VisualLink
              className="card post-card"
              href={s.siteHref("/blog/five-jobs")}
              data-category="guides"
              hidden={!s.postVisible("guides")}
              effect="card"
            >
              <RevealFrame
                className="cover"
                style={{
                  background: "linear-gradient(135deg,#5A2AF1,#0EA5E9)",
                }}
              >
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M12 3l1.9 5.8L20 11l-6.1 2.2L12 19l-1.9-5.8L4 11l6.1-2.2z"></path>
                </svg>
              </RevealFrame>
              <div className="body">
                <span className="badge b-v" style={{ alignSelf: "flex-start" }}>
                  {"Guides"}
                </span>
                <h3>
                  {"Five repetitive jobs to hand off to an AI this month"}
                </h3>
                <p>
                  {
                    "Bookings, FAQs, callbacks, morning briefs and research: where to start, and what to keep for yourself."
                  }
                </p>
                <div className="meta">
                  <span>{"Sep 18, 2026"}</span>
                  <span>{"·"}</span>
                  <span>{"7 min read"}</span>
                </div>
              </div>
            </VisualLink>
            <VisualLink
              className="card post-card"
              href={s.siteHref("/blog/scoped-keys")}
              data-category="dev"
              hidden={!s.postVisible("dev")}
              effect="card"
            >
              <RevealFrame
                className="cover"
                style={{
                  background: "linear-gradient(135deg,#27272A,#5A2AF1)",
                }}
              >
                <svg className="i" viewBox="0 0 24 24">
                  <circle cx="7.5" cy="15.5" r="5.5"></circle>
                  <path d="m21 2-9.6 9.6M15.5 7.5l3 3L22 7l-3-3"></path>
                </svg>
              </RevealFrame>
              <div className="body">
                <span className="badge b-v" style={{ alignSelf: "flex-start" }}>
                  {"Developers"}
                </span>
                <h3>{"Scoped keys: give agents only what they need"}</h3>
                <p>
                  {
                    "Agent Keys can be narrowed to specific services and operations, with optional input limits. Here’s how."
                  }
                </p>
                <div className="meta">
                  <span>{"Sep 9, 2026"}</span>
                  <span>{"·"}</span>
                  <span>{"8 min read"}</span>
                </div>
              </div>
            </VisualLink>
            <VisualLink
              className="card post-card"
              href={s.siteHref("/blog/x-dms-live")}
              data-category="updates"
              hidden={!s.postVisible("updates")}
              effect="card"
            >
              <RevealFrame
                className="cover"
                style={{
                  background: "linear-gradient(135deg,#18181B,#3F3F46)",
                }}
              >
                <svg className="i" viewBox="0 0 24 24">
                  <path d="m3 11 18-5v12L3 14v-3z"></path>
                  <path d="M11.6 16.8a3 3 0 1 1-5.8-1.6"></path>
                </svg>
              </RevealFrame>
              <div className="body">
                <span className="badge b-v" style={{ alignSelf: "flex-start" }}>
                  {"Product updates"}
                </span>
                <h3>{"Product update: X (Twitter) DMs are live"}</h3>
                <p>
                  {
                    "Your assistant can now answer direct messages on X, with the same limited keys and approvals as Telegram."
                  }
                </p>
                <div className="meta">
                  <span>{"Aug 28, 2026"}</span>
                  <span>{"·"}</span>
                  <span>{"2 min read"}</span>
                </div>
              </div>
            </VisualLink>
          </div>
          {"\n  "}
          <div
            className="card"
            style={{
              marginTop: "48px",
              display: "flex",
              justifyContent: "space-between",
              alignItems: "center",
              gap: "20px",
              flexWrap: "wrap",
            }}
          >
            <div>
              <b style={{ fontSize: "18px" }}>{"Get one useful idea a week"}</b>
              <p className="small">{"No spam. Unsubscribe anytime."}</p>
            </div>
            <div
              className="nl-form"
              style={{ display: "flex", gap: "8px", flexWrap: "wrap" }}
            >
              <input
                placeholder="you@business.com"
                style={{
                  height: "38px",
                  borderRadius: "9px",
                  border: "1px solid var(--border2)",
                  background: "var(--bg)",
                  color: "var(--fg)",
                  padding: "0 12px",
                  font: "inherit",
                  fontSize: "14px",
                  width: "240px",
                }}
                disabled={true}
              />
              <button className="btn" disabled={true} type="button">
                {"Subscriptions coming soon"}
              </button>
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
                {" Discord"}
              </a>
            </div>
          </div>
          {"\n"}
        </div>
      </section>
    </>
  );
}
