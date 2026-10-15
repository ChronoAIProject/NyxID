// Generated from approved A, SHA-256 5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33.
// Run: node scripts/website-a/generate.mjs <baseline.html>
import { DemoAgents, DemoMessages, DemoChips } from "../../interactions";
import { useWebsite } from "../../website-context";
export default function NyxbotPage() {
  const s = useWebsite();
  void s;
  return (
    <>
      {"\n"}
      <section className="s nb-hero" style={{ position: "relative" }}>
        {"\n  "}
        <div className="wrap nb-split">
          {"\n    "}
          <div>
            {"\n      "}
            <div className="nb-tag">
              <span className="nb-av">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M12 3l1.9 5.8L20 11l-6.1 2.2L12 19l-1.9-5.8L4 11l6.1-2.2z"></path>
                </svg>
              </span>
              {"NyxBot "}
              <span className="small" style={{ marginLeft: "6px" }}>
                {"on NyxID"}
              </span>
            </div>
            {"\n      "}
            <h1
              className="h1"
              style={{ fontSize: "var(--fs-display)", marginTop: "22px" }}
            >
              {"Just ask."}
              <br />
              <span className="grad">{"NyxBot handles it."}</span>
            </h1>
            {"\n      "}
            <p className="lead" style={{ marginTop: "18px" }}>
              {
                "The easy way to use NyxID. Tell it what you need in the app or on Telegram: it connects your services, sets what each assistant may do and automates the busywork. It isn’t a general chatbot; it works on your apps."
              }
            </p>
            {"\n      "}
            <div className="nb-try">
              <div className="small" style={{ marginBottom: "10px" }}>
                {"Try asking"}
              </div>
              <DemoChips />
            </div>
            {"\n      "}
            <div
              style={{
                display: "flex",
                gap: "12px",
                flexWrap: "wrap",
                marginTop: "30px",
              }}
            >
              <a className="btn p lg" href={s.authHref("register")}>
                {"Start with NyxBot"}
              </a>
              <a className="btn lg" href={s.siteHref("/solutions")}>
                {"See what it can run"}
              </a>
            </div>
            {"\n    "}
          </div>
          {"\n    "}
          <div
            className="nb-app"
            onMouseEnter={s.stopDemo}
            onFocusCapture={s.stopDemo}
          >
            {"\n      "}
            <div className="nb-bar">
              <i></i>
              <i></i>
              <i></i>
              <span>{"NyxID · Assistant"}</span>
            </div>
            {"\n      "}
            <div className="nb-body">
              {"\n        "}
              <DemoAgents />
              {"\n        "}
              <div className="nb-thread">
                {"\n          "}
                <div className="nb-th">
                  <span className="nb-av sm">
                    <svg className="i" viewBox="0 0 24 24">
                      <path d="M12 3l1.9 5.8L20 11l-6.1 2.2L12 19l-1.9-5.8L4 11l6.1-2.2z"></path>
                    </svg>
                  </span>
                  <div>
                    <b>{"NyxBot"}</b>
                    <div className="small" style={{ fontSize: "11.5px" }}>
                      {"Your assistant · NyxMode"}
                    </div>
                  </div>
                </div>
                {"\n          "}
                <DemoMessages />
                {"\n          "}
                <div className="nb-comp">
                  <span>{"Message NyxBot…"}</span>
                  <span className="nb-send">
                    <svg className="i" viewBox="0 0 24 24">
                      <path d="M22 2 11 13"></path>
                      <path d="M22 2 15 22l-4-9-9-4 20-7z"></path>
                    </svg>
                  </span>
                </div>
                {"\n        "}
              </div>
              {"\n      "}
            </div>
            {"\n    "}
          </div>
          {"\n  "}
        </div>
        {"\n"}
      </section>
      {"\n"}
      <section id="try-nyxbot" style={{ padding: "8px 0 24px" }}>
        {"\n  "}
        <div className="wrap">
          <div className="try-strip">
            <span className="try-tag">{"Connect NyxBot"}</span>
            <span className="try-txt">
              {"Sign in to connect your services and set up your assistant."}
            </span>
            <span className="try-links">
              <a className="btn sm" href={s.authHref("register")}>
                {"Get started"}
              </a>
            </span>
          </div>
        </div>
        {"\n"}
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap">
          {"\n  "}
          <div
            className="center"
            style={{
              textAlign: "center",
              maxWidth: "720px",
              marginBottom: "40px",
            }}
          >
            <div className="over v">{"Things you can say"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"Talk to it like a colleague."}
            </h2>
            <p className="lead center" style={{ marginTop: "14px" }}>
              {
                "No menus to learn. Say what you want; NyxBot works out the services, the permissions and the schedule."
              }
            </p>
          </div>
          {"\n  "}
          <div className="grid g2">
            <div className="card say">
              <div className="q">
                {"“Answer my customers on Telegram while I’m closed.”"}
              </div>
              <div className="a">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
                <span>
                  {
                    "Creates a Help desk specialist and links it to your Telegram bot."
                  }
                </span>
              </div>
              <div className="pills" style={{ gap: "6px", marginTop: "14px" }}>
                <span className="badge b-m">{"Telegram"}</span>
                <span className="badge b-m">{"Sheets · read-only"}</span>
              </div>
            </div>
            <div className="card say">
              <div className="q">
                {"“How many bookings did we get last week?”"}
              </div>
              <div className="a">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
                <span>{"Reads your calendar and replies with a summary."}</span>
              </div>
              <div className="pills" style={{ gap: "6px", marginTop: "14px" }}>
                <span className="badge b-m">{"Calendar · read-only"}</span>
              </div>
            </div>
            <div className="card say">
              <div className="q">{"“Connect my Gmail.”"}</div>
              <div className="a">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
                <span>
                  {
                    "Sends a secure sign-in link. It never asks for your password."
                  }
                </span>
              </div>
              <div className="pills" style={{ gap: "6px", marginTop: "14px" }}>
                <span className="badge b-m">{"Secure link"}</span>
              </div>
            </div>
            <div className="card say">
              <div className="q">
                {"“Every Monday, email the team a list of no-shows.”"}
              </div>
              <div className="a">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
                <span>
                  {"Creates a weekly schedule and asks before the first send."}
                </span>
              </div>
              <div className="pills" style={{ gap: "6px", marginTop: "14px" }}>
                <span className="badge b-m">{"Calendar"}</span>
                <span className="badge b-m">{"Gmail"}</span>
                <span className="badge b-m">{"Weekly"}</span>
              </div>
            </div>
            <div className="card say">
              <div className="q">
                {"“Download this month’s invoices from the supplier portal.”"}
              </div>
              <div className="a">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
                <span>
                  {
                    "Uses the browser on your computer with the saved login hidden."
                  }
                </span>
              </div>
              <div className="pills" style={{ gap: "6px", marginTop: "14px" }}>
                <span className="badge b-m">{"Computer · browser"}</span>
                <span className="badge b-m">{"Files"}</span>
              </div>
            </div>
            <div className="card say">
              <div className="q">{"“Let the researcher use Google Docs.”"}</div>
              <div className="a">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
                <span>
                  {
                    "Grants one service to one specialist, after you approve the change."
                  }
                </span>
              </div>
              <div className="pills" style={{ gap: "6px", marginTop: "14px" }}>
                <span className="badge b-m">{"Grant"}</span>
                <span className="badge b-m">{"Your OK"}</span>
              </div>
            </div>
          </div>
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s glowbg">
        {"\n  "}
        <div className="wrap split">
          {"\n    "}
          <div>
            {"\n      "}
            <div className="over v">{"Computer use"}</div>
            {"\n      "}
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"No API? "}
              <span className="grad">{"It uses a computer."}</span>
            </h2>
            {"\n      "}
            <p className="lead" style={{ marginTop: "14px" }}>
              {
                "Supplier portals, government forms, old booking systems: lots of work only happens in a browser. Connect a computer and NyxBot can click, type, download and file, just like a person at a desk."
              }
            </p>
            {"\n      "}
            <div className="grid g2" style={{ marginTop: "26px" }}>
              {"\n        "}
              <div className="card" style={{ padding: "18px" }}>
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <rect x="2" y="3" width="20" height="14" rx="2"></rect>
                    <path d="M8 21h8M12 17v4"></path>
                  </svg>
                </div>
                <b style={{ display: "block", margin: "12px 0 4px" }}>
                  {"Your own computer"}
                </b>
                <p className="small" style={{ fontSize: "13.5px" }}>
                  {
                    "A Mac or Linux machine in your office. Install the NyxID app once; it connects securely, with no open ports."
                  }
                </p>
              </div>
              {"\n        "}
              <div className="card" style={{ padding: "18px" }}>
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M17.5 19H9a7 7 0 1 1 6.71-9h1.79a4.5 4.5 0 1 1 0 9Z"></path>
                  </svg>
                </div>
                <b style={{ display: "block", margin: "12px 0 4px" }}>
                  {"A hosted VM"}
                </b>
                <p className="small" style={{ fontSize: "13.5px" }}>
                  {
                    "A separate desktop in the cloud, kept apart from your own files. Best for always-on work."
                  }
                </p>
              </div>
              {"\n      "}
            </div>
            {"\n    "}
          </div>
          {"\n    "}
          <div className="desk mock">
            {"\n  "}
            <div className="tb">
              <i></i>
              <i></i>
              <i></i>
              <span style={{ marginLeft: "8px" }}>
                {"Mac mini · live desktop"}
              </span>
              <span className="live">{"NyxBot is working"}</span>
            </div>
            {"\n  "}
            <div className="addr">
              <svg className="i" viewBox="0 0 24 24">
                <rect x="3" y="11" width="18" height="11" rx="2"></rect>
                <path d="M7 11V7a5 5 0 0 1 10 0v4"></path>
              </svg>
              {"portal.freshfarms.example/orders/new"}
            </div>
            {"\n  "}
            <div className="pg">
              <h4>{"Fresh Farms · New order"}</h4>
              {"\n    "}
              <div className="line">
                <span>{"Roma tomatoes, case"}</span>
                <span>{"× 12"}</span>
                <span>{"$186.00"}</span>
              </div>
              <div className="line">
                <span>{"Basil, bunch"}</span>
                <span>{"× 6"}</span>
                <span>{"$21.00"}</span>
              </div>
              <div className="line">
                <span>{"Delivery · Thursday AM"}</span>
                <span></span>
                <span>{"free"}</span>
              </div>
              {"\n    "}
              <div className="tot">
                <b>{"Total $207.00"}</b>
                <span className="btn p sm">{"Place order"}</span>
              </div>
              {"\n  "}
            </div>
            {"\n  "}
            <svg className="cursor" viewBox="0 0 24 24">
              <path
                d="M4 3l7.07 17 2.51-7.39L21 10.07z"
                fill="currentColor"
              ></path>
            </svg>
            {"\n  "}
            <div className="foot">
              <span>
                <svg className="i" viewBox="0 0 24 24">
                  <circle cx="7.5" cy="15.5" r="5.5"></circle>
                  <path d="m21 2-9.6 9.6M15.5 7.5l3 3L22 7l-3-3"></path>
                </svg>
                {" Saved login filled · password hidden from AI"}
              </span>
              <span className="btn sm">{"Take control"}</span>
            </div>
            {"\n"}
          </div>
          {"\n  "}
        </div>
        {"\n"}
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap">
          {"\n  "}
          <div className="center" style={{ textAlign: "center" }}>
            <div className="over v">{"Switched on one by one"}</div>
            <h2
              className="h2"
              style={{ marginTop: "10px", marginBottom: "36px" }}
            >
              {"You decide what it can touch"}
            </h2>
          </div>
          {"\n  "}
          <div className="grid g4">
            <div className="card">
              <div
                style={{
                  display: "flex",
                  justifyContent: "space-between",
                  alignItems: "flex-start",
                }}
              >
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <rect x="2" y="3" width="20" height="18" rx="2"></rect>
                    <path d="M2 9h20M6 6h.01M9 6h.01"></path>
                  </svg>
                </div>
                <span className="badge b-ok">{"Allowed"}</span>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"Browser"}
              </h3>
              <p className="sub" style={{ fontSize: "14px" }}>
                {"Opens websites, clicks and fills forms in a secure browser."}
              </p>
            </div>
            <div className="card">
              <div
                style={{
                  display: "flex",
                  justifyContent: "space-between",
                  alignItems: "flex-start",
                }}
              >
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M4 20h16a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.93a2 2 0 0 1-1.66-.9l-.82-1.2A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13c0 1.1.9 2 2 2Z"></path>
                  </svg>
                </div>
                <span className="badge b-ok">{"Allowed"}</span>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"Files"}
              </h3>
              <p className="sub" style={{ fontSize: "14px" }}>
                {"Reads and saves files only inside the folders you choose."}
              </p>
            </div>
            <div className="card">
              <div
                style={{
                  display: "flex",
                  justifyContent: "space-between",
                  alignItems: "flex-start",
                }}
              >
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <rect x="2" y="3" width="20" height="14" rx="2"></rect>
                    <path d="M8 21h8M12 17v4"></path>
                  </svg>
                </div>
                <span className="badge b-ask">{"Ask me first"}</span>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"Desktop control"}
              </h3>
              <p className="sub" style={{ fontSize: "14px" }}>
                {"Uses desktop apps with mouse and keyboard when needed."}
              </p>
            </div>
            <div className="card">
              <div
                style={{
                  display: "flex",
                  justifyContent: "space-between",
                  alignItems: "flex-start",
                }}
              >
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <polyline points="4 17 10 11 4 5"></polyline>
                    <line x1="12" y1="19" x2="20" y2="19"></line>
                  </svg>
                </div>
                <span className="badge b-no">{"Not allowed"}</span>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"Shell"}
              </h3>
              <p className="sub" style={{ fontSize: "14px" }}>
                {"Runs commands. Off unless you turn it on."}
              </p>
            </div>
          </div>
          {"\n  "}
          <div className="grid g3" style={{ marginTop: "16px" }}>
            <div className="card">
              <div className="ic">
                <svg className="i" viewBox="0 0 24 24">
                  <circle cx="7.5" cy="15.5" r="5.5"></circle>
                  <path d="m21 2-9.6 9.6M15.5 7.5l3 3L22 7l-3-3"></path>
                </svg>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"Saved logins stay hidden"}
              </h3>
              <p className="sub" style={{ fontSize: "14px" }}>
                {
                  "Passwords are encrypted and filled straight into the exact website. The AI drives the page but never reads them."
                }
              </p>
            </div>
            <div className="card">
              <div className="ic">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M2 12s3-7 10-7 10 7 10 7-3 7-10 7-10-7-10-7Z"></path>
                  <circle cx="12" cy="12" r="3"></circle>
                </svg>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"Watch live, take over anytime"}
              </h3>
              <p className="sub" style={{ fontSize: "14px" }}>
                {
                  "Open the live desktop from your browser. One click and you’re in control."
                }
              </p>
            </div>
            <div className="card">
              <div className="ic">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"></path>
                  <path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"></path>
                </svg>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"Confirms before it acts"}
              </h3>
              <p className="sub" style={{ fontSize: "14px" }}>
                {"Orders, form submissions and file changes wait for your OK."}
              </p>
            </div>
          </div>
          {"\n  "}
          <div style={{ textAlign: "center", marginTop: "28px" }}>
            <a
              className="btn"
              href={s.siteHref("/solutions/back-office-clerk")}
            >
              {"See the Back-office clerk solution "}
              <svg className="i" viewBox="0 0 24 24">
                <path d="M5 12h14M12 5l7 7-7 7"></path>
              </svg>
            </a>
          </div>
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s glowbg">
        <div className="wrap">
          {"\n  "}
          <div
            className="center"
            style={{ textAlign: "center", maxWidth: "720px" }}
          >
            <div className="over v">{"NyxBot + specialists"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"A team, not a single bot"}
            </h2>
            <p className="lead center" style={{ marginTop: "14px" }}>
              {
                "NyxBot is the one you talk to. Each specialist gets only the services its job needs: your restaurant manager can book tables, but it can’t read your email."
              }
            </p>
          </div>
          {"\n  "}
          <div className="org">
            {"\n    "}
            <div className="org-top">
              <span className="nb-av">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M12 3l1.9 5.8L20 11l-6.1 2.2L12 19l-1.9-5.8L4 11l6.1-2.2z"></path>
                </svg>
              </span>
              <div>
                <b>{"NyxBot"}</b>
                <div className="small">
                  {"Talks to you · creates and manages specialists"}
                </div>
              </div>
            </div>
            {"\n    "}
            <div className="org-stem"></div>
            <div className="org-rail"></div>
            {"\n    "}
            <div className="grid g4 org-grid">
              <a
                className="card clk"
                href={s.siteHref("/solutions/restaurant-manager")}
              >
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M3 2v7c0 1.1.9 2 2 2h4a2 2 0 0 0 2-2V2"></path>
                    <path d="M7 2v20"></path>
                    <path d="M21 15V2a5 5 0 0 0-5 5v6c0 1.1.9 2 2 2h3Zm0 0v7"></path>
                  </svg>
                </div>
                <b
                  style={{
                    display: "block",
                    margin: "12px 0 10px",
                    fontSize: "15px",
                  }}
                >
                  {"Restaurant manager"}
                </b>
                <div className="pills" style={{ gap: "6px" }}>
                  <span className="badge b-m">{"Telegram"}</span>
                  <span className="badge b-m">{"Google Calendar"}</span>
                  <span className="badge b-m">{"Google Sheets"}</span>
                </div>
                <div
                  className="small"
                  style={{ marginTop: "12px", color: "var(--err)" }}
                >
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M18 6 6 18M6 6l12 12"></path>
                  </svg>
                  {" Can’t: charge deposits"}
                </div>
              </a>
              <a
                className="card clk"
                href={s.siteHref("/solutions/news-aggregator")}
              >
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M4 22h16a2 2 0 0 0 2-2V4a2 2 0 0 0-2-2H8a2 2 0 0 0-2 2v16a2 2 0 0 1-2 2Zm0 0a2 2 0 0 1-2-2v-9c0-1.1.9-2 2-2h2"></path>
                    <path d="M18 14h-8"></path>
                    <path d="M15 18h-5"></path>
                    <path d="M10 6h8v4h-8V6Z"></path>
                  </svg>
                </div>
                <b
                  style={{
                    display: "block",
                    margin: "12px 0 10px",
                    fontSize: "15px",
                  }}
                >
                  {"News aggregator"}
                </b>
                <div className="pills" style={{ gap: "6px" }}>
                  <span className="badge b-m">{"Firecrawl"}</span>
                  <span className="badge b-m">{"X (Twitter)"}</span>
                  <span className="badge b-m">{"Tavily"}</span>
                  <span className="badge b-m">{"Telegram"}</span>
                </div>
                <div className="small" style={{ marginTop: "12px" }}>
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M2 12s3-7 10-7 10 7 10 7-3 7-10 7-10-7-10-7Z"></path>
                    <circle cx="12" cy="12" r="3"></circle>
                  </svg>
                  {" Read-only sources"}
                </div>
              </a>
              <a
                className="card clk"
                href={s.siteHref("/solutions/back-office-clerk")}
              >
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <rect x="2" y="3" width="20" height="14" rx="2"></rect>
                    <path d="M8 21h8M12 17v4"></path>
                  </svg>
                </div>
                <b
                  style={{
                    display: "block",
                    margin: "12px 0 10px",
                    fontSize: "15px",
                  }}
                >
                  {"Back-office clerk"}
                </b>
                <div className="pills" style={{ gap: "6px" }}>
                  <span className="badge b-m">{"Computer · browser"}</span>
                  <span className="badge b-m">{"Computer · files"}</span>
                  <span className="badge b-m">{"Google Sheets"}</span>
                  <span className="badge b-m">{"Computer · checkout"}</span>
                </div>
                <div
                  className="small"
                  style={{ marginTop: "12px", color: "var(--err)" }}
                >
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M18 6 6 18M6 6l12 12"></path>
                  </svg>
                  {" Can’t: run commands"}
                </div>
              </a>
              <a
                className="card clk"
                href={s.siteHref("/solutions/callback-queue")}
              >
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M22 16.92v3a2 2 0 0 1-2.18 2 19.79 19.79 0 0 1-8.63-3.07 19.5 19.5 0 0 1-6-6 19.79 19.79 0 0 1-3.07-8.67A2 2 0 0 1 4.11 2h3a2 2 0 0 1 2 1.72c.13.96.36 1.9.7 2.81a2 2 0 0 1-.45 2.11L8.09 9.91a16 16 0 0 0 6 6l1.27-1.27a2 2 0 0 1 2.11-.45c.91.34 1.85.57 2.81.7A2 2 0 0 1 22 16.92z"></path>
                  </svg>
                </div>
                <b
                  style={{
                    display: "block",
                    margin: "12px 0 10px",
                    fontSize: "15px",
                  }}
                >
                  {"Callback queue"}
                </b>
                <div className="pills" style={{ gap: "6px" }}>
                  <span className="badge b-m">{"Google Sheets"}</span>
                  <span className="badge b-m">{"Twilio"}</span>
                  <span className="badge b-m">{"ElevenLabs"}</span>
                  <span className="badge b-m">{"Discounts & refunds"}</span>
                </div>
                <div
                  className="small"
                  style={{ marginTop: "12px", color: "var(--warn)" }}
                >
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"></path>
                    <path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"></path>
                  </svg>
                  {" Asks first: offer money back"}
                </div>
              </a>
            </div>
            {"\n  "}
          </div>
          {"\n"}
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap" style={{ maxWidth: "860px" }}>
          <div className="over v">{"FAQ"}</div>
          <h2 className="h2" style={{ margin: "10px 0 18px" }}>
            {"About NyxBot"}
          </h2>
          <details>
            <summary>
              {"Is NyxBot a ChatGPT, Claude or Grok replacement?"}
            </summary>
            <p>
              {
                "No. Keep using whichever AI you like for questions and writing. NyxBot is for running your services: connecting, controlling and automating them. You can even connect ChatGPT or Claude to NyxID."
              }
            </p>
          </details>
          <details>
            <summary>{"Is NyxBot different from NyxID?"}</summary>
            <p>
              {
                "NyxID is the product: the secure gateway for logins, scoped keys, approvals and logs. NyxBot is the friendly way to use it in plain words. Developers can use the same gateway directly through the API or MCP."
              }
            </p>
          </details>
          <details>
            <summary>{"Can my customers talk to NyxBot?"}</summary>
            <p>
              {
                "Customers talk to specialists you link to a Telegram or X bot. They only get what you allow for guests, and never your computer."
              }
            </p>
          </details>
          <details>
            <summary>{"What does it need to use my computer?"}</summary>
            <p>
              {
                "Install the NyxID app on the machine (or use a hosted VM), pair it with your account, then switch on only the capabilities you want."
              }
            </p>
          </details>
          <details>
            <summary>{"Can I use my own AI agent instead?"}</summary>
            <p>
              {
                "Yes. Connect any agent through the API or MCP and it gets the same safe access."
              }
            </p>
          </details>
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap">
          <div className="band">
            <h2 className="h2">{"Say hello to NyxBot."}</h2>
            <p style={{ opacity: ".88", marginTop: "12px", fontSize: "17px" }}>
              {"Get started and ask it to set up your first job."}
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
              <a className="btn lg" href={s.authHref("register")}>
                {"Get started"}
              </a>
              <a className="btn lg gh" href={s.siteHref("/solutions")}>
                {"Browse solutions"}
              </a>
            </div>
          </div>
        </div>
      </section>
    </>
  );
}
