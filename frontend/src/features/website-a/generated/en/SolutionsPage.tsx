// Generated from approved A, SHA-256 5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33.
// Run: node scripts/website-a/generate.mjs <baseline.html>
import { SiteLink } from "../../site-link";
import { SolutionFilm } from "../../interactions";
import { VisualLink } from "../../visuals/elements";
import { useWebsite } from "../../website-context";
export default function SolutionsPage() {
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
          <div className="over v">{"Solutions"}</div>
          <h1
            className="h1"
            style={{ marginTop: "14px", fontSize: "var(--fs-h1)" }}
          >
            {"What should your assistant do?"}
          </h1>
          <p className="lead">
            {
              "Ready-made jobs for real businesses, with the right connections and safe permissions already set."
            }
          </p>
          <div
            className="pills"
            style={{ marginTop: "22px", justifyContent: "center" }}
          >
            <span className="badge b-ok">{"● Platform prices, no markup"}</span>
            <span className="badge b-v">{"Custom bundles for business"}</span>
          </div>
        </div>
        {"\n"}
      </section>
      {"\n"}
      <section className="s" style={{ paddingTop: "8px", border: "none" }}>
        <div className="wrap">
          <div className="center" style={{ textAlign: "center" }}>
            <div className="over v">{"See it work"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"Five jobs, one assistant"}
            </h2>
          </div>
          <SolutionFilm />
        </div>
      </section>
      {"\n"}
      <section className="s" style={{ paddingTop: "40px" }}>
        {"\n  "}
        <div className="wrap">
          {"\n    "}
          <div
            style={{
              display: "flex",
              justifyContent: "space-between",
              gap: "16px",
              flexWrap: "wrap",
              marginBottom: "28px",
            }}
          >
            {"\n      "}
            <div className="pills" id="catF">
              <button
                className={s.filterClass("catF", "all")}
                data-v="all"
                type="button"
                onClick={() => s.setFilter("catF", "all")}
                aria-pressed={s.isFilterSelected("catF", "all")}
              >
                {"All"}
              </button>
              <button
                className={s.filterClass("catF", "hosp")}
                data-v="hosp"
                type="button"
                onClick={() => s.setFilter("catF", "hosp")}
                aria-pressed={s.isFilterSelected("catF", "hosp")}
              >
                {"Hospitality"}
              </button>
              <button
                className={s.filterClass("catF", "support")}
                data-v="support"
                type="button"
                onClick={() => s.setFilter("catF", "support")}
                aria-pressed={s.isFilterSelected("catF", "support")}
              >
                {"Customer support"}
              </button>
              <button
                className={s.filterClass("catF", "research")}
                data-v="research"
                type="button"
                onClick={() => s.setFilter("catF", "research")}
                aria-pressed={s.isFilterSelected("catF", "research")}
              >
                {"Research & insights"}
              </button>
              <button
                className={s.filterClass("catF", "sales")}
                data-v="sales"
                type="button"
                onClick={() => s.setFilter("catF", "sales")}
                aria-pressed={s.isFilterSelected("catF", "sales")}
              >
                {"Sales & marketing"}
              </button>
              <button
                className={s.filterClass("catF", "ops")}
                data-v="ops"
                type="button"
                onClick={() => s.setFilter("catF", "ops")}
                aria-pressed={s.isFilterSelected("catF", "ops")}
              >
                {"Operations"}
              </button>
            </div>
            {"\n      "}
            <div className="pills" id="chF">
              <span
                className="small"
                style={{ alignSelf: "center", marginRight: "4px" }}
              >
                {"Channel"}
              </span>
              <button
                className={s.filterClass("chF", "Telegram")}
                data-v="Telegram"
                type="button"
                onClick={() => s.setFilter("chF", "Telegram")}
                aria-pressed={s.isFilterSelected("chF", "Telegram")}
              >
                {"Telegram"}
              </button>
              <button
                className={s.filterClass("chF", "X")}
                data-v="X"
                type="button"
                onClick={() => s.setFilter("chF", "X")}
                aria-pressed={s.isFilterSelected("chF", "X")}
              >
                {"X"}
              </button>
              <button
                className={s.filterClass("chF", "Phone")}
                data-v="Phone"
                type="button"
                onClick={() => s.setFilter("chF", "Phone")}
                aria-pressed={s.isFilterSelected("chF", "Phone")}
              >
                {"Phone"}
              </button>
              <button
                className={s.filterClass("chF", "Computer")}
                data-v="Computer"
                type="button"
                onClick={() => s.setFilter("chF", "Computer")}
                aria-pressed={s.isFilterSelected("chF", "Computer")}
              >
                {"Computer"}
              </button>
            </div>
            {"\n    "}
          </div>
          {"\n    "}
          <div className="grid g3" id="solGrid">
            <VisualLink
              className="card sol feat"
              href={s.siteHref("/solutions/restaurant-manager")}
              data-category="hosp"
              data-channels="Telegram"
              hidden={!s.solutionVisible("hosp", "Telegram")}
              effect="card"
            >
              {"\n  "}
              <div className="top">
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M3 2v7c0 1.1.9 2 2 2h4a2 2 0 0 0 2-2V2"></path>
                    <path d="M7 2v20"></path>
                    <path d="M21 15V2a5 5 0 0 0-5 5v6c0 1.1.9 2 2 2h3Zm0 0v7"></path>
                  </svg>
                </div>
                <span className="badge b-v">{"Most popular"}</span>
              </div>
              {"\n  "}
              <h3>{"Restaurant manager"}</h3>
              <p>
                {
                  "Takes table bookings in Telegram, checks your calendar, notes allergies and sends reminders."
                }
              </p>
              {"\n  "}
              <div className="tags">
                <span className="badge b-ok">{"● Telegram"}</span>
              </div>
              {"\n  "}
              <div className="small" style={{ fontSize: "12px" }}>
                {"Platform prices · no markup"}
              </div>
              {"\n  "}
              <div className="foot">
                <span>
                  <svg className="i" viewBox="0 0 24 24">
                    <rect x="3" y="11" width="18" height="11" rx="2"></rect>
                    <path d="M7 11V7a5 5 0 0 1 10 0v4"></path>
                  </svg>
                  {"Never: charge deposits"}
                </span>
                <span className="more">
                  {"View "}
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M5 12h14M12 5l7 7-7 7"></path>
                  </svg>
                </span>
              </div>
            </VisualLink>
            <VisualLink
              className="card sol "
              href={s.siteHref("/solutions/help-desk")}
              data-category="support"
              data-channels="Telegram,X"
              hidden={!s.solutionVisible("support", "Telegram,X")}
              effect="card"
            >
              {"\n  "}
              <div className="top">
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M3 14h3a2 2 0 0 1 2 2v3a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-7a9 9 0 0 1 18 0v7a2 2 0 0 1-2 2h-1a2 2 0 0 1-2-2v-3a2 2 0 0 1 2-2h3"></path>
                  </svg>
                </div>
                <span className="badge b-m">{"Customer support"}</span>
              </div>
              {"\n  "}
              <h3>{"24-hour help desk"}</h3>
              <p>
                {
                  "Answers order, delivery and policy questions any time, and hands tricky ones to your team."
                }
              </p>
              {"\n  "}
              <div className="tags">
                <span className="badge b-ok">{"● Telegram"}</span>
                <span className="badge b-ok">{"● X"}</span>
              </div>
              {"\n  "}
              <div className="small" style={{ fontSize: "12px" }}>
                {"Platform prices · no markup"}
              </div>
              {"\n  "}
              <div className="foot">
                <span>
                  <svg className="i" viewBox="0 0 24 24">
                    <rect x="3" y="11" width="18" height="11" rx="2"></rect>
                    <path d="M7 11V7a5 5 0 0 1 10 0v4"></path>
                  </svg>
                  {"Never: issue refunds"}
                </span>
                <span className="more">
                  {"View "}
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M5 12h14M12 5l7 7-7 7"></path>
                  </svg>
                </span>
              </div>
            </VisualLink>
            <VisualLink
              className="card sol "
              href={s.siteHref("/solutions/appointment-booker")}
              data-category="hosp"
              data-channels="Telegram"
              hidden={!s.solutionVisible("hosp", "Telegram")}
              effect="card"
            >
              {"\n  "}
              <div className="top">
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <circle cx="6" cy="6" r="3"></circle>
                    <circle cx="6" cy="18" r="3"></circle>
                    <path d="M20 4 8.12 15.88M14.47 14.48 20 20M8.12 8.12 12 12"></path>
                  </svg>
                </div>
                <span className="badge b-m">{"Hospitality"}</span>
              </div>
              {"\n  "}
              <h3>{"Appointment booker"}</h3>
              <p>
                {
                  "Offers open slots, books the right person, and handles reschedules with your OK."
                }
              </p>
              {"\n  "}
              <div className="tags">
                <span className="badge b-ok">{"● Telegram"}</span>
              </div>
              {"\n  "}
              <div className="small" style={{ fontSize: "12px" }}>
                {"Platform prices · no markup"}
              </div>
              {"\n  "}
              <div className="foot">
                <span>
                  <svg className="i" viewBox="0 0 24 24">
                    <rect x="3" y="11" width="18" height="11" rx="2"></rect>
                    <path d="M7 11V7a5 5 0 0 1 10 0v4"></path>
                  </svg>
                  {"Never: move appointments"}
                </span>
                <span className="more">
                  {"View "}
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M5 12h14M12 5l7 7-7 7"></path>
                  </svg>
                </span>
              </div>
            </VisualLink>
            <VisualLink
              className="card sol "
              href={s.siteHref("/solutions/callback-queue")}
              data-category="support"
              data-channels="Phone"
              hidden={!s.solutionVisible("support", "Phone")}
              effect="card"
            >
              {"\n  "}
              <div className="top">
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M22 16.92v3a2 2 0 0 1-2.18 2 19.79 19.79 0 0 1-8.63-3.07 19.5 19.5 0 0 1-6-6 19.79 19.79 0 0 1-3.07-8.67A2 2 0 0 1 4.11 2h3a2 2 0 0 1 2 1.72c.13.96.36 1.9.7 2.81a2 2 0 0 1-.45 2.11L8.09 9.91a16 16 0 0 0 6 6l1.27-1.27a2 2 0 0 1 2.11-.45c.91.34 1.85.57 2.81.7A2 2 0 0 1 22 16.92z"></path>
                  </svg>
                </div>
                <span className="badge b-m">{"Customer support"}</span>
              </div>
              {"\n  "}
              <h3>{"Callback queue"}</h3>
              <p>
                {
                  "Works through your list of customers to call back, has the conversation with an AI voice, and logs the outcome."
                }
              </p>
              {"\n  "}
              <div className="tags">
                <span className="badge b-ro">{"● Phone"}</span>
              </div>
              {"\n  "}
              <div className="small" style={{ fontSize: "12px" }}>
                {"Platform prices · no markup"}
              </div>
              {"\n  "}
              <div className="foot">
                <span>
                  <svg className="i" viewBox="0 0 24 24">
                    <rect x="3" y="11" width="18" height="11" rx="2"></rect>
                    <path d="M7 11V7a5 5 0 0 1 10 0v4"></path>
                  </svg>
                  {"Never: offer money back"}
                </span>
                <span className="more">
                  {"View "}
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M5 12h14M12 5l7 7-7 7"></path>
                  </svg>
                </span>
              </div>
            </VisualLink>
            <VisualLink
              className="card sol "
              href={s.siteHref("/solutions/stock-analyst")}
              data-category="research"
              data-channels="Telegram"
              hidden={!s.solutionVisible("research", "Telegram")}
              effect="card"
            >
              {"\n  "}
              <div className="top">
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <polyline points="22 7 13.5 15.5 8.5 10.5 2 17"></polyline>
                    <polyline points="16 7 22 7 22 13"></polyline>
                  </svg>
                </div>
                <span className="badge b-m">{"Research & insights"}</span>
              </div>
              {"\n  "}
              <h3>{"Stock analyst"}</h3>
              <p>
                {
                  "Watches your watchlist, scans market chatter and briefs you in plain English. Reads, never trades."
                }
              </p>
              {"\n  "}
              <div className="tags">
                <span className="badge b-ok">{"● Telegram"}</span>
              </div>
              {"\n  "}
              <div className="small" style={{ fontSize: "12px" }}>
                {"Platform prices · no markup"}
              </div>
              {"\n  "}
              <div className="foot">
                <span>
                  <svg className="i" viewBox="0 0 24 24">
                    <rect x="3" y="11" width="18" height="11" rx="2"></rect>
                    <path d="M7 11V7a5 5 0 0 1 10 0v4"></path>
                  </svg>
                  {"Never: place trades"}
                </span>
                <span className="more">
                  {"View "}
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M5 12h14M12 5l7 7-7 7"></path>
                  </svg>
                </span>
              </div>
            </VisualLink>
            <VisualLink
              className="card sol "
              href={s.siteHref("/solutions/news-aggregator")}
              data-category="research"
              data-channels="Telegram"
              hidden={!s.solutionVisible("research", "Telegram")}
              effect="card"
            >
              {"\n  "}
              <div className="top">
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M4 22h16a2 2 0 0 0 2-2V4a2 2 0 0 0-2-2H8a2 2 0 0 0-2 2v16a2 2 0 0 1-2 2Zm0 0a2 2 0 0 1-2-2v-9c0-1.1.9-2 2-2h2"></path>
                    <path d="M18 14h-8"></path>
                    <path d="M15 18h-5"></path>
                    <path d="M10 6h8v4h-8V6Z"></path>
                  </svg>
                </div>
                <span className="badge b-m">{"Research & insights"}</span>
              </div>
              {"\n  "}
              <h3>{"News aggregator"}</h3>
              <p>
                {
                  "Reads the sources you care about every morning and sends one tidy brief without duplicates."
                }
              </p>
              {"\n  "}
              <div className="tags">
                <span className="badge b-ok">{"● Telegram"}</span>
              </div>
              {"\n  "}
              <div className="small" style={{ fontSize: "12px" }}>
                {"Platform prices · no markup"}
              </div>
              {"\n  "}
              <div className="foot">
                <span>
                  <svg className="i" viewBox="0 0 24 24">
                    <rect x="3" y="11" width="18" height="11" rx="2"></rect>
                    <path d="M7 11V7a5 5 0 0 1 10 0v4"></path>
                  </svg>
                  {"Read-only"}
                </span>
                <span className="more">
                  {"View "}
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M5 12h14M12 5l7 7-7 7"></path>
                  </svg>
                </span>
              </div>
            </VisualLink>
            <VisualLink
              className="card sol "
              href={s.siteHref("/solutions/desk-researcher")}
              data-category="research"
              data-channels="Telegram"
              hidden={!s.solutionVisible("research", "Telegram")}
              effect="card"
            >
              {"\n  "}
              <div className="top">
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <circle cx="11" cy="11" r="8"></circle>
                    <path d="m21 21-4.3-4.3"></path>
                  </svg>
                </div>
                <span className="badge b-m">{"Research & insights"}</span>
              </div>
              {"\n  "}
              <h3>{"Desk researcher"}</h3>
              <p>
                {
                  "Searches, reads and compares, then writes a finished report into your Google Docs."
                }
              </p>
              {"\n  "}
              <div className="tags">
                <span className="badge b-ok">{"● Telegram"}</span>
              </div>
              {"\n  "}
              <div className="small" style={{ fontSize: "12px" }}>
                {"Platform prices · no markup"}
              </div>
              {"\n  "}
              <div className="foot">
                <span>
                  <svg className="i" viewBox="0 0 24 24">
                    <rect x="3" y="11" width="18" height="11" rx="2"></rect>
                    <path d="M7 11V7a5 5 0 0 1 10 0v4"></path>
                  </svg>
                  {"Never: delete files"}
                </span>
                <span className="more">
                  {"View "}
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M5 12h14M12 5l7 7-7 7"></path>
                  </svg>
                </span>
              </div>
            </VisualLink>
            <VisualLink
              className="card sol "
              href={s.siteHref("/solutions/x-lead-catcher")}
              data-category="sales"
              data-channels="X"
              hidden={!s.solutionVisible("sales", "X")}
              effect="card"
            >
              {"\n  "}
              <div className="top">
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="m3 11 18-5v12L3 14v-3z"></path>
                    <path d="M11.6 16.8a3 3 0 1 1-5.8-1.6"></path>
                  </svg>
                </div>
                <span className="badge b-m">{"Sales & marketing"}</span>
              </div>
              {"\n  "}
              <h3>{"X lead catcher"}</h3>
              <p>
                {
                  "Replies to product questions in your X DMs, qualifies the lead and adds it to your sheet."
                }
              </p>
              {"\n  "}
              <div className="tags">
                <span className="badge b-ok">{"● X"}</span>
              </div>
              {"\n  "}
              <div className="small" style={{ fontSize: "12px" }}>
                {"Platform prices · no markup"}
              </div>
              {"\n  "}
              <div className="foot">
                <span>
                  <svg className="i" viewBox="0 0 24 24">
                    <rect x="3" y="11" width="18" height="11" rx="2"></rect>
                    <path d="M7 11V7a5 5 0 0 1 10 0v4"></path>
                  </svg>
                  {"Never: post publicly"}
                </span>
                <span className="more">
                  {"View "}
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M5 12h14M12 5l7 7-7 7"></path>
                  </svg>
                </span>
              </div>
            </VisualLink>
            <VisualLink
              className="card sol "
              href={s.siteHref("/solutions/back-office-clerk")}
              data-category="ops"
              data-channels="Telegram,Computer"
              hidden={!s.solutionVisible("ops", "Telegram,Computer")}
              effect="card"
            >
              {"\n  "}
              <div className="top">
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <rect x="2" y="3" width="20" height="14" rx="2"></rect>
                    <path d="M8 21h8M12 17v4"></path>
                  </svg>
                </div>
                <span className="badge b-m">{"Operations"}</span>
              </div>
              {"\n  "}
              <h3>{"Back-office clerk"}</h3>
              <p>
                {
                  "Logs in to supplier and booking portals that have no API, reorders stock, downloads invoices and files them, on your computer or a hosted VM."
                }
              </p>
              {"\n  "}
              <div className="tags">
                <span className="badge b-ok">{"● Telegram"}</span>
                <span className="badge b-ro">{"● Computer"}</span>
              </div>
              {"\n  "}
              <div className="small" style={{ fontSize: "12px" }}>
                {"Platform prices · no markup"}
              </div>
              {"\n  "}
              <div className="foot">
                <span>
                  <svg className="i" viewBox="0 0 24 24">
                    <rect x="3" y="11" width="18" height="11" rx="2"></rect>
                    <path d="M7 11V7a5 5 0 0 1 10 0v4"></path>
                  </svg>
                  {"Never: run commands"}
                </span>
                <span className="more">
                  {"View "}
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M5 12h14M12 5l7 7-7 7"></path>
                  </svg>
                </span>
              </div>
            </VisualLink>
            <VisualLink
              className="card sol"
              href={s.siteHref("/nyxbot")}
              style={{
                borderStyle: "dashed",
                alignItems: "center",
                justifyContent: "center",
                textAlign: "center",
              }}
              effect="card"
            >
              <div className="ic">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M12 3l1.9 5.8L20 11l-6.1 2.2L12 19l-1.9-5.8L4 11l6.1-2.2z"></path>
                </svg>
              </div>
              <h3>{"Describe your own"}</h3>
              <p>
                {
                  "“When someone asks for a quote, collect their address and email it to me.”"
                }
              </p>
              <span className="btn sm" style={{ marginTop: "6px" }}>
                {"Ask NyxBot"}
              </span>
            </VisualLink>
          </div>
          {"\n  "}
        </div>
        {"\n"}
      </section>
      {"\n"}
      <section className="s glowbg">
        <div className="wrap split">
          {"\n  "}
          <div>
            <div className="over v">{"For business & enterprise"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"Running this at scale? "}
              <span className="grad">{"Get a custom bundle."}</span>
            </h2>
            <p className="lead" style={{ marginTop: "14px" }}>
              {
                "Every solution runs at platform prices with no markup. For teams that need predictable spend, volume or a specific deployment, we put together a bundle around your workload."
              }
            </p>
            <div
              style={{
                display: "flex",
                gap: "10px",
                flexWrap: "wrap",
                marginTop: "24px",
              }}
            >
              <VisualLink
                className="btn p"
                href={s.authHref("register")}
                effect="button"
              >
                {"Get started"}
              </VisualLink>
              <SiteLink
                className="btn"
                href={s.siteHref("/contact?topic=bundle")}
              >
                {"Talk to us"}
              </SiteLink>
            </div>
          </div>
          {"\n  "}
          <div className="card" style={{ padding: "26px" }}>
            {"\n    "}
            <div className="over">{"A bundle can include"}</div>
            {"\n    "}
            <ul className="checks" style={{ marginTop: "18px", gap: "12px" }}>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M20 6 9 17l-5-5"></path>
                  </svg>
                </span>
                <div>
                  <b style={{ fontSize: "14.5px", fontWeight: "500" }}>
                    {"Predictable monthly spend across your solutions"}
                  </b>
                </div>
              </li>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M20 6 9 17l-5-5"></path>
                  </svg>
                </span>
                <div>
                  <b style={{ fontSize: "14.5px", fontWeight: "500" }}>
                    {"Volume for high-traffic channels and agents"}
                  </b>
                </div>
              </li>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M20 6 9 17l-5-5"></path>
                  </svg>
                </span>
                <div>
                  <b style={{ fontSize: "14.5px", fontWeight: "500" }}>
                    {"SSO, org roles and admin controls"}
                  </b>
                </div>
              </li>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M20 6 9 17l-5-5"></path>
                  </svg>
                </span>
                <div>
                  <b style={{ fontSize: "14.5px", fontWeight: "500" }}>
                    {"On-prem credential nodes or self-hosted deployment"}
                  </b>
                </div>
              </li>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M20 6 9 17l-5-5"></path>
                  </svg>
                </span>
                <div>
                  <b style={{ fontSize: "14.5px", fontWeight: "500" }}>
                    {"Priority support and onboarding"}
                  </b>
                </div>
              </li>
            </ul>
            {"\n  "}
          </div>
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap" style={{ maxWidth: "900px" }}>
          {"\n  "}
          <div className="center" style={{ textAlign: "center" }}>
            <div className="over v">{"A day with your solutions"}</div>
            <h2
              className="h2"
              style={{ marginTop: "10px", marginBottom: "36px" }}
            >
              {"While you get on with your day"}
            </h2>
          </div>
          {"\n  "}
          <div className="tl">
            <div className="t">
              <div className="card">
                <span className="time">{"08:00"}</span>
                <div>
                  <b>{"Sends your morning brief"}</b>
                  <div className="small">{"News aggregator"}</div>
                </div>
                <span className="badge b-ok">{"Done"}</span>
              </div>
            </div>
            <div className="t">
              <div className="card">
                <span className="time">{"11:20"}</span>
                <div>
                  <b>{"Answers “do you have parking?”"}</b>
                  <div className="small">{"24-hour help desk"}</div>
                </div>
                <span className="badge b-ok">{"Done"}</span>
              </div>
            </div>
            <div className="t">
              <div className="card">
                <span className="time">{"14:10"}</span>
                <div>
                  <b>{"Reorders produce on the supplier portal"}</b>
                  <div className="small">{"Back-office clerk"}</div>
                </div>
                <span className="badge b-ask">{"Your OK"}</span>
              </div>
            </div>
            <div className="t">
              <div className="card">
                <span className="time">{"15:05"}</span>
                <div>
                  <b>{"Calls back 6 customers: 3 booked"}</b>
                  <div className="small">{"Callback queue"}</div>
                </div>
                <span className="badge b-ok">{"Done"}</span>
              </div>
            </div>
            <div className="t">
              <div className="card">
                <span className="time">{"23:42"}</span>
                <div>
                  <b>{"Books a table for 4 after closing"}</b>
                  <div className="small">{"Restaurant manager"}</div>
                </div>
                <span className="badge b-ok">{"Done"}</span>
              </div>
            </div>
          </div>
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s">
        {"\n  "}
        <div className="wrap">
          {"\n    "}
          <div className="over v">{"By industry"}</div>
          <h2
            className="h2"
            style={{ marginTop: "10px", marginBottom: "32px" }}
          >
            {"Popular combinations"}
          </h2>
          {"\n    "}
          <div className="grid g4">
            {"\n      "}
            <div className="card">
              <div className="ic">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M3 2v7c0 1.1.9 2 2 2h4a2 2 0 0 0 2-2V2"></path>
                  <path d="M7 2v20"></path>
                  <path d="M21 15V2a5 5 0 0 0-5 5v6c0 1.1.9 2 2 2h3Zm0 0v7"></path>
                </svg>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 10px" }}>
                {"Restaurants & cafés"}
              </h3>
              <SiteLink
                className="more"
                style={{ display: "flex", padding: "4px 0" }}
                href={s.siteHref("/solutions/restaurant-manager")}
              >
                {"Restaurant manager "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </SiteLink>
              <SiteLink
                className="more"
                style={{ display: "flex", padding: "4px 0" }}
                href={s.siteHref("/solutions/back-office-clerk")}
              >
                {"Back-office clerk "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </SiteLink>
              <SiteLink
                className="more"
                style={{ display: "flex", padding: "4px 0" }}
                href={s.siteHref("/solutions/callback-queue")}
              >
                {"Callback queue "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </SiteLink>
            </div>
            <div className="card">
              <div className="ic">
                <svg className="i" viewBox="0 0 24 24">
                  <circle cx="6" cy="6" r="3"></circle>
                  <circle cx="6" cy="18" r="3"></circle>
                  <path d="M20 4 8.12 15.88M14.47 14.48 20 20M8.12 8.12 12 12"></path>
                </svg>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 10px" }}>
                {"Salons & clinics"}
              </h3>
              <SiteLink
                className="more"
                style={{ display: "flex", padding: "4px 0" }}
                href={s.siteHref("/solutions/appointment-booker")}
              >
                {"Appointment booker "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </SiteLink>
              <SiteLink
                className="more"
                style={{ display: "flex", padding: "4px 0" }}
                href={s.siteHref("/solutions/callback-queue")}
              >
                {"Callback queue "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </SiteLink>
              <SiteLink
                className="more"
                style={{ display: "flex", padding: "4px 0" }}
                href={s.siteHref("/solutions/help-desk")}
              >
                {"24-hour help desk "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </SiteLink>
            </div>
            <div className="card">
              <div className="ic">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"></path>
                </svg>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 10px" }}>
                {"Online shops"}
              </h3>
              <SiteLink
                className="more"
                style={{ display: "flex", padding: "4px 0" }}
                href={s.siteHref("/solutions/help-desk")}
              >
                {"24-hour help desk "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </SiteLink>
              <SiteLink
                className="more"
                style={{ display: "flex", padding: "4px 0" }}
                href={s.siteHref("/solutions/x-lead-catcher")}
              >
                {"X lead catcher "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </SiteLink>
              <SiteLink
                className="more"
                style={{ display: "flex", padding: "4px 0" }}
                href={s.siteHref("/solutions/back-office-clerk")}
              >
                {"Back-office clerk "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </SiteLink>
            </div>
            <div className="card">
              <div className="ic">
                <svg className="i" viewBox="0 0 24 24">
                  <polyline points="22 7 13.5 15.5 8.5 10.5 2 17"></polyline>
                  <polyline points="16 7 22 7 22 13"></polyline>
                </svg>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 10px" }}>
                {"Founders & analysts"}
              </h3>
              <SiteLink
                className="more"
                style={{ display: "flex", padding: "4px 0" }}
                href={s.siteHref("/solutions/stock-analyst")}
              >
                {"Stock analyst "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </SiteLink>
              <SiteLink
                className="more"
                style={{ display: "flex", padding: "4px 0" }}
                href={s.siteHref("/solutions/news-aggregator")}
              >
                {"News aggregator "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </SiteLink>
              <SiteLink
                className="more"
                style={{ display: "flex", padding: "4px 0" }}
                href={s.siteHref("/solutions/desk-researcher")}
              >
                {"Desk researcher "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </SiteLink>
            </div>
            {"\n    "}
          </div>
          {"\n  "}
        </div>
        {"\n"}
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap">
          <div className="band">
            <h2 className="h2">{"Don’t see your job?"}</h2>
            <p style={{ opacity: ".88", marginTop: "12px", fontSize: "17px" }}>
              {
                "Describe it in a sentence and NyxBot will set up a custom assistant."
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
