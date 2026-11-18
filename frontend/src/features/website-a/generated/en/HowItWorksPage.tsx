// Generated from approved A, SHA-256 5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33.
// Run: node scripts/website-a/generate.mjs <baseline.html>
import { SiteLink } from "../../site-link";
import { VisualLink, RevealFrame } from "../../visuals/elements";
import { useWebsite } from "../../website-context";
export default function HowItWorksPage() {
  const s = useWebsite();
  void s;
  return (
    <>
      {"\n"}
      <section className="hero glowbg" style={{ position: "relative" }}>
        {"\n  "}
        <div className="wrap">
          <div className="over v">{"How it works"}</div>
          <h1
            className="h1"
            style={{ marginTop: "14px", fontSize: "var(--fs-h1)" }}
          >
            {"How NyxID connects AI"}
            <br />
            {"to your world, "}
            <span className="grad">{"safely."}</span>
          </h1>
          <p className="lead">
            {
              "Four layers, one rule: the AI only ever gets what you allow. Here’s what happens between “take this booking” and “done”."
            }
          </p>
        </div>
        {"\n"}
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap">
          {"\n  "}
          <div
            className="center"
            style={{ textAlign: "center", maxWidth: "720px" }}
          >
            <div className="over v">{"The big picture"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"Four layers"}
            </h2>
          </div>
          {"\n  "}
          <div
            className="grid g4"
            style={{ marginTop: "40px", position: "relative" }}
          >
            {"\n    "}
            <div className="card" style={{}}>
              <div className="ic ">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"></path>
                </svg>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"1 · Where you ask"}
              </h3>
              <p className="sub" style={{ fontSize: "14px" }}>
                {
                  "Telegram, X DMs, the NyxID app, or your own app through the API or MCP."
                }
              </p>
              <div className="pills" style={{ marginTop: "14px", gap: "6px" }}>
                <span className="badge b-m">{"Telegram"}</span>
                <span className="badge b-m">{"X"}</span>
                <span className="badge b-m">{"App"}</span>
                <span className="badge b-m">{"API · MCP"}</span>
              </div>
            </div>
            <div className="card" style={{}}>
              <div className="ic ">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M12 3l1.9 5.8L20 11l-6.1 2.2L12 19l-1.9-5.8L4 11l6.1-2.2z"></path>
                </svg>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"2 · NyxBot thinks"}
              </h3>
              <p className="sub" style={{ fontSize: "14px" }}>
                {
                  "Understands the request, picks the right specialist and plans the steps."
                }
              </p>
              <div className="pills" style={{ marginTop: "14px", gap: "6px" }}>
                <span className="badge b-m">{"Specialists"}</span>
                <span className="badge b-m">{"Memory"}</span>
                <span className="badge b-m">{"Schedules"}</span>
              </div>
            </div>
            <div
              className="card"
              style={{
                borderColor: "var(--accentb)",
                boxShadow: "0 0 0 3px var(--accent)",
              }}
            >
              <div className="ic g">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"></path>
                </svg>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"3 · NyxID checks"}
              </h3>
              <p className="sub" style={{ fontSize: "14px" }}>
                {
                  "The gateway checks the scoped key, asks you if needed, adds the hidden login and records the call."
                }
              </p>
              <div className="pills" style={{ marginTop: "14px", gap: "6px" }}>
                <span className="badge b-m">{"Vault"}</span>
                <span className="badge b-m">{"Scoped keys"}</span>
                <span className="badge b-m">{"Approvals"}</span>
                <span className="badge b-m">{"Audit log"}</span>
              </div>
            </div>
            <div className="card" style={{}}>
              <div className="ic ">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M12 22v-5M9 8V2M15 8V2M18 8v5a6 6 0 0 1-12 0V8z"></path>
                </svg>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"4 · It gets done"}
              </h3>
              <p className="sub" style={{ fontSize: "14px" }}>
                {
                  "Calls the service’s API, an AI model, or uses a browser on your computer or a hosted VM."
                }
              </p>
              <div className="pills" style={{ marginTop: "14px", gap: "6px" }}>
                <span className="badge b-m">{"100+ APIs"}</span>
                <span className="badge b-m">{"AI models"}</span>
                <span className="badge b-m">{"Computers"}</span>
              </div>
            </div>
            {"\n  "}
          </div>
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap split">
          {"\n  "}
          <div>
            <div className="over v">{"Life of a request"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"What happens in about a second"}
            </h2>
            <p className="lead" style={{ marginTop: "14px" }}>
              {
                "Every single action, whether booking a table or clicking a button on a website, goes through the same checks."
              }
            </p>
          </div>
          {"\n  "}
          <div
            className="card log"
            style={{ fontFamily: "var(--sans)", fontSize: "14px" }}
          >
            <div className="r">
              <span className="tm mono">{"01"}</span>
              <span>{"NyxBot asks to create a calendar event"}</span>
              <span className="badge b-m">{"request"}</span>
            </div>
            <div className="r">
              <span className="tm mono">{"02"}</span>
              <span>{"Is this key allowed to create events?"}</span>
              <span className="badge b-ok">{"yes"}</span>
            </div>
            <div className="r">
              <span className="tm mono">{"03"}</span>
              <span>{"Does this action need your OK?"}</span>
              <span className="badge b-ro">{"no"}</span>
            </div>
            <div className="r">
              <span className="tm mono">{"04"}</span>
              <span>{"Add the hidden Google login"}</span>
              <span className="badge b-v">{"vault"}</span>
            </div>
            <div className="r">
              <span className="tm mono">{"05"}</span>
              <span>{"Send to Google Calendar"}</span>
              <span className="badge b-ok">{"200 OK"}</span>
            </div>
            <div className="r">
              <span className="tm mono">{"06"}</span>
              <span>{"Record it in the tamper-evident log"}</span>
              <span className="badge b-ok">{"logged"}</span>
            </div>
          </div>
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap split">
          {"\n  "}
          <div>
            <div className="stepn">{"1"}</div>
            <h2
              className="h3"
              style={{ margin: "16px 0 10px", fontSize: "var(--fs-h2-sub)" }}
            >
              {"Tell NyxBot what you want"}
            </h2>
            <p className="lead">
              {
                "In plain words, in the NyxID app or Telegram. NyxBot works out which services it needs and what it should never touch."
              }
            </p>
            <SiteLink
              className="more"
              style={{ marginTop: "18px" }}
              href={s.siteHref("/solutions")}
            >
              {"Or start from a ready-made solution "}
              <svg className="i" viewBox="0 0 24 24">
                <path d="M5 12h14M12 5l7 7-7 7"></path>
              </svg>
            </SiteLink>
          </div>
          {"\n  "}
          <div className="phone">
            <div className="ph-h">
              <div className="av">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M12 3l1.9 5.8L20 11l-6.1 2.2L12 19l-1.9-5.8L4 11l6.1-2.2z"></path>
                </svg>
              </div>
              <div>
                <b style={{ fontSize: "14px" }}>{"NyxBot"}</b>
                <div className="small" style={{ fontSize: "11.5px" }}>
                  {"Your assistant"}
                </div>
              </div>
            </div>
            <div className="msgs">
              <div className="m in">
                {
                  "Answer customer questions on Telegram and look up orders in my sheet. Don’t ever give refunds."
                }
              </div>
              <div className="m out">
                {
                  "Got it. I’ll create a Help desk specialist: Telegram replies allowed, your order sheet read-only, refunds blocked. Connect your sheet here:"
                }
              </div>
              <div className="m tool">
                {"🔗 Secure sign-in link · Google Sheets"}
              </div>
            </div>
          </div>
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap split rev">
          {"\n  "}
          <div>
            <div className="stepn">{"2"}</div>
            <h2
              className="h3"
              style={{ margin: "16px 0 10px", fontSize: "var(--fs-h2-sub)" }}
            >
              {"Connect your tools, safely"}
            </h2>
            <p className="lead">
              {
                "You sign in to Google, Telegram or X on their own pages, just like any “Sign in with Google” button. NyxID keeps the login locked in an encrypted vault. The AI never sees it. For websites with no API, connect a computer or hosted VM."
              }
            </p>
            {"\n  "}
            <ul className="checks" style={{ marginTop: "22px" }}>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <rect x="3" y="11" width="18" height="11" rx="2"></rect>
                    <path d="M7 11V7a5 5 0 0 1 10 0v4"></path>
                  </svg>
                </span>
                <div>
                  <b>{"Encrypted vault"}</b>
                  <span>
                    {"AES-256 encryption, with cloud key management options."}
                  </span>
                </div>
              </li>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <rect x="2" y="3" width="20" height="14" rx="2"></rect>
                    <path d="M8 21h8M12 17v4"></path>
                  </svg>
                </span>
                <div>
                  <b>{"Computer or hosted VM"}</b>
                  <span>{"For portals and apps without an API."}</span>
                </div>
              </li>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M18.36 6.64a9 9 0 1 1-12.73 0"></path>
                    <path d="M12 2v10"></path>
                  </svg>
                </span>
                <div>
                  <b>{"Disconnect any time"}</b>
                  <span>{"One click removes the connection everywhere."}</span>
                </div>
              </li>
            </ul>
          </div>
          {"\n  "}
          <div
            className="card mock"
            style={{
              maxWidth: "420px",
              width: "100%",
              margin: "0 auto",
              padding: "28px",
              textAlign: "center",
            }}
          >
            {"\n    "}
            <div
              style={{
                display: "flex",
                justifyContent: "center",
                alignItems: "center",
                gap: "14px",
              }}
            >
              <span className="gl" style={{ background: "#4285F4" }}>
                {"31"}
              </span>
              <span className="small">{"⟷"}</span>
              <img
                src="/website-a/fa92f171d1d90a9d.svg"
                width="30"
                onError={s.imageFallback}
              />
            </div>
            {"\n    "}
            <h3 style={{ fontSize: "var(--fs-h3)", margin: "18px 0 6px" }}>
              {"Connect Google Calendar"}
            </h3>
            <p className="small">
              {"Luna Bistro’s assistant will be able to:"}
            </p>
            {"\n    "}
            <div
              style={{ textAlign: "left", margin: "16px 0" }}
              className="grid"
            >
              <div
                style={{
                  display: "flex",
                  justifyContent: "space-between",
                  fontSize: "14px",
                  padding: "6px 0",
                  borderBottom: "1px solid var(--border)",
                }}
              >
                {"See free times"}
                <span className="badge b-ok">{"Allowed"}</span>
              </div>
              <div
                style={{
                  display: "flex",
                  justifyContent: "space-between",
                  fontSize: "14px",
                  padding: "6px 0",
                  borderBottom: "1px solid var(--border)",
                }}
              >
                {"Create bookings"}
                <span className="badge b-ok">{"Allowed"}</span>
              </div>
              <div
                style={{
                  display: "flex",
                  justifyContent: "space-between",
                  fontSize: "14px",
                  padding: "6px 0",
                  borderBottom: "1px solid var(--border)",
                }}
              >
                {"Cancel bookings"}
                <span className="badge b-ask">{"Ask me first"}</span>
              </div>
            </div>
            {"\n    "}
            <button className="btn p" style={{ width: "100%" }} type="button">
              {"Sign in with Google"}
            </button>
            <p className="small" style={{ marginTop: "10px" }}>
              {"Secure link · expires in 15 minutes"}
            </p>
            {"\n  "}
          </div>
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap split">
          {"\n  "}
          <div>
            <div className="stepn">{"3"}</div>
            <h2
              className="h3"
              style={{ margin: "16px 0 10px", fontSize: "var(--fs-h2-sub)" }}
            >
              {"Set the rules"}
            </h2>
            <p className="lead">
              {
                "Every action gets one of four settings. Solutions come with safe defaults, so most owners never change a thing."
              }
            </p>
            {"\n  "}
            <div className="grid g2" style={{ marginTop: "24px" }}>
              <div className="card" style={{ padding: "14px" }}>
                <span className="badge b-ok">{"Allowed"}</span>
                <p className="small" style={{ marginTop: "8px" }}>
                  {"Does it on its own"}
                </p>
              </div>
              <div className="card" style={{ padding: "14px" }}>
                <span className="badge b-ro">{"Read-only"}</span>
                <p className="small" style={{ marginTop: "8px" }}>
                  {"Can look, not change"}
                </p>
              </div>
              <div className="card" style={{ padding: "14px" }}>
                <span className="badge b-ask">{"Ask me first"}</span>
                <p className="small" style={{ marginTop: "8px" }}>
                  {"Sends you a yes/no first"}
                </p>
              </div>
              <div className="card" style={{ padding: "14px" }}>
                <span className="badge b-no">{"Not allowed"}</span>
                <p className="small" style={{ marginTop: "8px" }}>
                  {"Can never do it"}
                </p>
              </div>
            </div>
          </div>
          {"\n  "}
          <div className="perm-card">
            <div className="pc-h">
              <div>
                <b>{"Restaurant manager"}</b>
                <div className="small">{"What this assistant can do"}</div>
              </div>
              <span className="badge b-v">
                <svg className="i" viewBox="0 0 24 24">
                  <circle cx="7.5" cy="15.5" r="5.5"></circle>
                  <path d="m21 2-9.6 9.6M15.5 7.5l3 3L22 7l-3-3"></path>
                </svg>
                {" Scoped key"}
              </span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#229ED9" }}>
                {"TG"}
              </span>
              <div>
                {"Telegram"}
                <small>{"reply to guests"}</small>
              </div>
              <span className="badge b-ok">{"Allowed"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#4285F4" }}>
                {"31"}
              </span>
              <div>
                {"Google Calendar"}
                <small>{"find & book tables"}</small>
              </div>
              <span className="badge b-ok">{"Allowed"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#0F9D58" }}>
                {"SH"}
              </span>
              <div>
                {"Google Sheets"}
                <small>{"guest notes & allergies"}</small>
              </div>
              <span className="badge b-ok">{"Allowed"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#4285F4" }}>
                {"31"}
              </span>
              <div>
                {"Google Calendar"}
                <small>{"cancel bookings"}</small>
              </div>
              <span className="badge b-ask">{"Ask me first"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#52525B" }}>
                {"$"}
              </span>
              <div>
                {"Payments"}
                <small>{"charge deposits"}</small>
              </div>
              <span className="badge b-no">{"Not allowed"}</span>
            </div>
          </div>
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap split rev">
          {"\n  "}
          <div>
            <div className="stepn">{"4"}</div>
            <h2
              className="h3"
              style={{ margin: "16px 0 10px", fontSize: "var(--fs-h2-sub)" }}
            >
              {"Go live where customers are"}
            </h2>
            <p className="lead">
              {
                "Create a Telegram bot in a few taps from inside Telegram, or connect your X account. Share the link on your website, Instagram bio or receipts."
              }
            </p>
            {"\n  "}
            <div className="pills" style={{ marginTop: "20px" }}>
              <span className="badge b-ok">{"● Telegram live"}</span>
              <span className="badge b-ok">{"● X DMs live"}</span>
              <span className="badge b-m">
                {"WhatsApp, Discord, Slack next"}
              </span>
            </div>
          </div>
          {"\n  "}
          <div className="phone">
            <div className="ph-h">
              <div className="av">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M3 2v7c0 1.1.9 2 2 2h4a2 2 0 0 0 2-2V2"></path>
                  <path d="M7 2v20"></path>
                  <path d="M21 15V2a5 5 0 0 0-5 5v6c0 1.1.9 2 2 2h3Zm0 0v7"></path>
                </svg>
              </div>
              <div>
                <b style={{ fontSize: "14px" }}>{"Luna Bistro"}</b>
                <div className="small" style={{ fontSize: "11.5px" }}>
                  {"Telegram · @LunaBistroBot"}
                </div>
              </div>
            </div>
            <div className="msgs">
              <div className="m sys">{"@LunaBistroBot · connected ✓"}</div>
              <div className="m in">{"Are you open on Sunday?"}</div>
              <div className="m out">
                {"Yes! Sunday 12–10 PM. Want me to book you a table?"}
              </div>
            </div>
          </div>
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap split">
          {"\n  "}
          <div>
            <div className="stepn">{"5"}</div>
            <h2
              className="h3"
              style={{ margin: "16px 0 10px", fontSize: "var(--fs-h2-sub)" }}
            >
              {"Stay in charge, from your phone"}
            </h2>
            <p className="lead">
              {
                "Approval cards for the risky stuff, a daily summary of what happened, and a full activity log whenever you want to look."
              }
            </p>
            {"\n  "}
            <ul className="checks" style={{ marginTop: "22px" }}>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"></path>
                    <path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"></path>
                  </svg>
                </span>
                <div>
                  <b>{"Approval cards"}</b>
                  <span>{"Tap yes or no in the NyxID app or Telegram."}</span>
                </div>
              </li>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M8 6h13M8 12h13M8 18h13M3 6h.01M3 12h.01M3 18h.01"></path>
                  </svg>
                </span>
                <div>
                  <b>{"Activity log"}</b>
                  <span>{"Every action, who did it and when."}</span>
                </div>
              </li>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M18.36 6.64a9 9 0 1 1-12.73 0"></path>
                    <path d="M12 2v10"></path>
                  </svg>
                </span>
                <div>
                  <b>{"Pause anytime"}</b>
                  <span>
                    {
                      "Disable an assistant in one click; turn it back on later."
                    }
                  </span>
                </div>
              </li>
            </ul>
          </div>
          {"\n  "}
          <div
            className="card mock"
            style={{
              maxWidth: "380px",
              width: "100%",
              margin: "0 auto",
              padding: "24px",
            }}
          >
            {"\n    "}
            <div style={{ display: "flex", alignItems: "center", gap: "10px" }}>
              <div className="ic">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"></path>
                  <path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"></path>
                </svg>
              </div>
              <div>
                <b>{"Approval needed"}</b>
                <div className="small">{"Restaurant manager · just now"}</div>
              </div>
            </div>
            {"\n    "}
            <p style={{ margin: "16px 0", fontSize: "15px" }}>
              {"Sarah wants to "}
              <b>{"cancel"}</b>
              {" Sun 7:30 PM (table for 4). Cancel it?"}
            </p>
            {"\n    "}
            <div className="grid g2">
              <button
                className="btn"
                style={{ color: "var(--err)" }}
                type="button"
              >
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M18 6 6 18M6 6l12 12"></path>
                </svg>
                {" Decline"}
              </button>
              <button
                className="btn"
                style={{ color: "var(--ok)" }}
                type="button"
              >
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M20 6 9 17l-5-5"></path>
                </svg>
                {" Approve"}
              </button>
            </div>
            {"\n    "}
            <p
              className="small"
              style={{ marginTop: "12px", textAlign: "center" }}
            >
              {"Code 4821 · expires in 10 min"}
            </p>
            {"\n  "}
          </div>
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap">
          {"\n  "}
          <div className="center" style={{ textAlign: "center" }}>
            <div className="over v">{"Three ways to use NyxID"}</div>
            <h2
              className="h2"
              style={{ marginTop: "10px", marginBottom: "36px" }}
            >
              {"Pick the one that fits you"}
            </h2>
          </div>
          {"\n  "}
          <RevealFrame className="grid g3">
            {"\n    "}
            <VisualLink
              className="card eq"
              href={s.siteHref("/nyxbot")}
              effect="card"
            >
              <span className="badge b-v">{"No code"}</span>
              <h3
                className="h3"
                style={{ fontSize: "var(--fs-h3)", margin: "14px 0 8px" }}
              >
                {"Ask NyxBot"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {
                  "Describe the job or pick a solution, connect Telegram, done. For owners and teams."
                }
              </p>
              <span className="more" style={{ marginTop: "16px" }}>
                {"Meet NyxBot "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </span>
            </VisualLink>
            {"\n    "}
            <VisualLink
              className="card eq"
              href={s.siteHref("/agents")}
              effect="card"
            >
              <span className="badge b-ro">{"Low code"}</span>
              <h3
                className="h3"
                style={{ fontSize: "var(--fs-h3)", margin: "14px 0 8px" }}
              >
                {"Your AI tool + NyxID"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {
                  "Plug Claude, ChatGPT or Cursor into NyxID with MCP and use your services safely."
                }
              </p>
              <span className="more" style={{ marginTop: "16px" }}>
                {"MCP setup "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </span>
            </VisualLink>
            {"\n    "}
            <VisualLink
              className="card eq"
              href={s.siteHref("/agents")}
              effect="card"
            >
              <span className="badge b-m">{"Your own code"}</span>
              <h3
                className="h3"
                style={{ fontSize: "var(--fs-h3)", margin: "14px 0 8px" }}
              >
                {"Call it from your app"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {
                  "Your script or app reaches 100+ services and AI models through NyxID with one scoped key."
                }
              </p>
              <span className="more" style={{ marginTop: "16px" }}>
                {"See how "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </span>
            </VisualLink>
            {"\n  "}
          </RevealFrame>
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap" style={{ maxWidth: "860px" }}>
          <div className="over v">{"FAQ"}</div>
          <h2 className="h2" style={{ margin: "10px 0 18px" }}>
            {"Common questions"}
          </h2>
          <details>
            <summary>{"Do I need to code?"}</summary>
            <p>
              {
                "No. Tell NyxBot what you need, or set up a solution with forms and plain-language instructions."
              }
            </p>
          </details>
          <details>
            <summary>{"How long does setup take?"}</summary>
            <p>
              {
                "Most solutions take a few minutes once your accounts are ready."
              }
            </p>
          </details>
          <details>
            <summary>{"What if the website I use has no API?"}</summary>
            <p>
              {
                "Connect a computer or a hosted VM and NyxBot can use the website in a browser, with your saved logins kept hidden."
              }
            </p>
          </details>
          <details>
            <summary>{"Which AI model does it use?"}</summary>
            <p>
              {
                "NyxID works with leading models. You can pick one per assistant."
              }
            </p>
          </details>
          <details>
            <summary>{"What happens if it gets something wrong?"}</summary>
            <p>
              {
                "Risky actions need your OK, everything is logged, and you can pause an assistant instantly."
              }
            </p>
          </details>
          <details>
            <summary>{"Can my team use it too?"}</summary>
            <p>
              {
                "Yes. Organisations let you share assistants and connections with role-based access."
              }
            </p>
          </details>
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap">
          <div className="band">
            <h2 className="h2">{"Hand off the repeat work tonight."}</h2>
            <p style={{ opacity: ".88", marginTop: "12px", fontSize: "17px" }}>
              {
                "Ask NyxBot to set up your first job, connect Telegram, and you’re live."
              }
            </p>
            <div
              style={{
                display: "flex",
                gap: "12px",
                justifyContent: "center",
                flexWrap: "wrap",
                marginTop: "28px",
                position: "relative",
                zIndex: "1",
              }}
            >
              <SiteLink className="btn lg" href={s.authHref("register")}>
                {"Get started"}
              </SiteLink>
              <SiteLink className="btn lg gh" href={s.siteHref("/solutions")}>
                {"Browse solutions"}
              </SiteLink>
            </div>
          </div>
        </div>
      </section>
    </>
  );
}
