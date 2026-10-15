// Generated from approved A, SHA-256 5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33.
// Run: node scripts/website-a/generate.mjs <baseline.html>
import { useWebsite } from "../../website-context";
export default function BlogLunaBistroPage() {
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
            <span>{"Customer stories"}</span>
          </div>
          <h1 className="h1" style={{ fontSize: "44px" }}>
            {"How a 40-seat bistro stopped missing late-night bookings"}
          </h1>
          <p className="lead" style={{ marginTop: "16px" }}>
            {
              "An illustrative story: after-hours messages used to wait until morning. Now they’re booked in seconds."
            }
          </p>
          <div className="author" style={{ marginTop: "22px" }}>
            <span className="a"></span>
            <div>
              <b>{"NyxID team"}</b>
              <div className="small">{"Oct 3, 2026 · 4 min read"}</div>
            </div>
          </div>
        </div>
        {"\n"}
      </section>
      {"\n"}
      <div className="wrap" style={{ maxWidth: "900px" }}>
        <div
          className="cover"
          style={{
            background: "linear-gradient(135deg,#5A2AF1,#B59FF9)",
            borderRadius: "20px",
            aspectRatio: "21/9",
          }}
        >
          <svg className="i" viewBox="0 0 24 24">
            <path d="M3 2v7c0 1.1.9 2 2 2h4a2 2 0 0 0 2-2V2"></path>
            <path d="M7 2v20"></path>
            <path d="M21 15V2a5 5 0 0 0-5 5v6c0 1.1.9 2 2 2h3Zm0 0v7"></path>
          </svg>
        </div>
      </div>
      {"\n"}
      <section className="s" style={{ paddingTop: "40px", border: "none" }}>
        <div className="wrap article">
          <div className="callout">
            {
              "Illustrative scenario showing how this works; not a customer case study."
            }
          </div>
          <p>
            {
              "Luna Bistro gets most booking requests between 10 PM and midnight, after the kitchen closes. By morning, half those guests had booked somewhere else."
            }
          </p>
          <h2>{"What changed"}</h2>
          <p>
            {
              "They asked NyxBot to set up a Restaurant manager, connected their Google Calendar and launched a Telegram bot linked from their Instagram bio."
            }
          </p>
          <ul>
            <li>{"Bookings are confirmed instantly, at any hour."}</li>
            <li>{"Allergies go straight into the guest sheet."}</li>
            <li>{"Cancellations still come to the owner for a one-tap OK."}</li>
          </ul>
          <blockquote>
            {
              "“It’s like hiring a night receptionist who never forgets to write things down.”"
            }
          </blockquote>
          <h2>{"What they kept in human hands"}</h2>
          <p>
            {
              "Deposits and refunds stay off. The assistant can book, but it can’t touch money."
            }
          </p>
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
            <a
              className="card post-card"
              href={s.siteHref("/blog/what-is-nyxid")}
            >
              <div
                className="cover"
                style={{
                  background: "linear-gradient(135deg,#A672FB,#5E00F5)",
                }}
              >
                <svg className="i" viewBox="0 0 24 24">
                  <path d="m12 2 10 5-10 5L2 7l10-5z"></path>
                  <path d="m2 17 10 5 10-5M2 12l10 5 10-5"></path>
                </svg>
              </div>
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
            </a>
            <a
              className="card post-card"
              href={s.siteHref("/blog/never-share-passwords")}
            >
              <div
                className="cover"
                style={{
                  background: "linear-gradient(135deg,#1E0A68,#5A2AF1)",
                }}
              >
                <svg className="i" viewBox="0 0 24 24">
                  <rect x="3" y="11" width="18" height="11" rx="2"></rect>
                  <path d="M7 11V7a5 5 0 0 1 10 0v4"></path>
                </svg>
              </div>
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
            </a>
            <a className="card post-card" href={s.siteHref("/blog/five-jobs")}>
              <div
                className="cover"
                style={{
                  background: "linear-gradient(135deg,#5A2AF1,#0EA5E9)",
                }}
              >
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M12 3l1.9 5.8L20 11l-6.1 2.2L12 19l-1.9-5.8L4 11l6.1-2.2z"></path>
                </svg>
              </div>
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
            </a>
          </div>
        </div>
      </section>
    </>
  );
}
