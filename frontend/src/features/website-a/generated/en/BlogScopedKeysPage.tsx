// Generated from approved A, SHA-256 5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33.
// Run: node scripts/website-a/generate.mjs <baseline.html>
import { VisualLink, RevealFrame } from "../../visuals/elements";
import { useWebsite } from "../../website-context";
export default function BlogScopedKeysPage() {
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
            <a href={s.siteHref("/blog")}>{"Blog"}</a>
            <span>{"/"}</span>
            <span>{"Developers"}</span>
          </div>
          <h1 className="h1" style={{ fontSize: "44px" }}>
            {"Scoped keys: give agents only what they need"}
          </h1>
          <p className="lead" style={{ marginTop: "16px" }}>
            {
              "Agent Keys can be narrowed to specific services and operations, with optional input limits. Here’s how."
            }
          </p>
          <div className="author" style={{ marginTop: "22px" }}>
            <span className="a"></span>
            <div>
              <b>{"NyxID engineering"}</b>
              <div className="small">{"Sep 9, 2026 · 8 min read"}</div>
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
            background: "linear-gradient(135deg,#27272A,#5A2AF1)",
            borderRadius: "20px",
            aspectRatio: "21/9",
          }}
        >
          <svg className="i" viewBox="0 0 24 24">
            <circle cx="7.5" cy="15.5" r="5.5"></circle>
            <path d="m21 2-9.6 9.6M15.5 7.5l3 3L22 7l-3-3"></path>
          </svg>
        </RevealFrame>
      </div>
      {"\n"}
      <section className="s" style={{ paddingTop: "40px", border: "none" }}>
        <div className="wrap article">
          <p>
            {
              "Every agent should run with its own key. NyxID Agent Keys (nyxid_ag_…) carry a service allowlist, per-key rate limits and audit attribution."
            }
          </p>
          <h2>{"Operation scopes"}</h2>
          <p>
            {
              "Go further than “which service”: allow specific operations such as calendar.create_event while denying calendar.delete_event. Optional input limits constrain paths, queries and bodies."
            }
          </p>
          <h2>{"Approvals"}</h2>
          <p>
            {
              "Pair scopes with approval modes so sensitive or destructive actions wait for a human."
            }
          </p>
          <div className="callout">
            {"Docs: Agent operation scopes and Agent isolation."}
          </div>
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
              <b>{"Ready to try it?"}</b>
              <div className="small">
                {"Ask NyxBot to set up your first job today."}
              </div>
            </div>
            <div style={{ display: "flex", gap: "8px", flexWrap: "wrap" }}>
              <a className="btn" href={s.siteHref("/solutions")}>
                {"Browse solutions"}
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
                {" Discuss on Discord"}
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
            <h2 className="h2">{"Keep reading"}</h2>
            <a className="btn" href={s.siteHref("/blog")}>
              {"All articles "}
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
                  {"Guides"}
                </span>
                <h3>
                  {
                    "What is an “agent gateway”? A plain-English guide for business owners"
                  }
                </h3>
                <p>
                  {
                    "You don’t need to rebuild anything to use AI. Here’s how a gateway sits between AI and the tools you already have."
                  }
                </p>
                <div className="meta">
                  <span>{"Oct 10, 2026"}</span>
                  <span>{"·"}</span>
                  <span>{"6 min read"}</span>
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
          </div>
        </div>
      </section>
    </>
  );
}
