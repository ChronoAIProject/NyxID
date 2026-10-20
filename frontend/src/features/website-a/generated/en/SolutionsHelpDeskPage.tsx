// Generated from approved A, SHA-256 5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33.
// Run: node scripts/website-a/generate.mjs <baseline.html>
import { VisualLink } from "../../visuals/elements";
import { SolutionActivation } from "../../solution-activation";
import { useWebsite } from "../../website-context";
export default function SolutionsHelpDeskPage() {
  const s = useWebsite();
  void s;
  return (
    <>
      {"\n"}
      <section
        className="hero glowbg"
        style={{
          textAlign: "left",
          paddingBottom: "64px",
          position: "relative",
        }}
      >
        {"\n  "}
        <div className="wrap split">
          {"\n    "}
          <div>
            {"\n      "}
            <div className="crumbs">
              <a href={s.siteHref("/solutions")}>{"Solutions"}</a>
              <span>{"/"}</span>
              <span>{"Customer support"}</span>
            </div>
            {"\n      "}
            <div className="ic lg g">
              <svg className="i" viewBox="0 0 24 24">
                <path d="M3 14h3a2 2 0 0 1 2 2v3a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-7a9 9 0 0 1 18 0v7a2 2 0 0 1-2 2h-1a2 2 0 0 1-2-2v-3a2 2 0 0 1 2-2h3"></path>
              </svg>
            </div>
            {"\n      "}
            <h1
              className="h1"
              style={{ fontSize: "var(--fs-h1)", marginTop: "20px" }}
            >
              {"24-hour help desk"}
            </h1>
            {"\n      "}
            <p className="lead" style={{ margin: "16px 0 0" }}>
              {
                "Answers order, delivery and policy questions any time, and hands tricky ones to your team."
              }
            </p>
            {"\n      "}
            <p
              style={{ marginTop: "14px", fontWeight: "600", fontSize: "17px" }}
            >
              {"Customers get answers at 3 AM. Your team sleeps."}
            </p>
            {"\n      "}
            <div
              style={{
                display: "flex",
                gap: "10px",
                flexWrap: "wrap",
                marginTop: "28px",
              }}
            >
              <SolutionActivation />
              <a className="btn lg" href={s.siteHref("/how-it-works")}>
                {"How it works"}
              </a>
            </div>
            {"\n      "}
            <div className="pills" style={{ marginTop: "20px" }}>
              <span className="badge b-ok">{"● Telegram"}</span>
              <span className="badge b-ok">{"● X"}</span>
              <span className="badge b-m">{"Telegram"}</span>
              <span className="badge b-m">{"X DMs"}</span>
              <span className="badge b-m">{"Google Sheets"}</span>
              <span className="badge b-m">{"Gmail"}</span>
              <span className="badge b-m">{"Payments"}</span>
            </div>
            {"\n    "}
          </div>
          {"\n    "}
          <div className="phone">
            <div className="ph-h">
              <div className="av">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M3 14h3a2 2 0 0 1 2 2v3a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-7a9 9 0 0 1 18 0v7a2 2 0 0 1-2 2h-1a2 2 0 0 1-2-2v-3a2 2 0 0 1 2-2h3"></path>
                </svg>
              </div>
              <div>
                <b style={{ fontSize: "14px" }}>{"24-hour help desk"}</b>
                <div className="small" style={{ fontSize: "11.5px" }}>
                  {"Telegram · assistant"}
                </div>
              </div>
            </div>
            <div className="msgs">
              <div className="m sys">{"Tue 3:12 AM"}</div>
              <div className="m in">
                {"My order #4821 still hasn’t arrived. Where is it?"}
              </div>
              <div className="m tool">{"✓ looked up order #4821"}</div>
              <div className="m out">
                {
                  "Sorry for the wait! #4821 shipped yesterday and arrives tomorrow by 6 PM. Tracking: KT-55210."
                }
              </div>
              <div className="m in">{"Can I get a refund if it is late?"}</div>
              <div className="m out">
                {"I’ve passed that to the team. They’ll reply by 10 AM."}
              </div>
            </div>
          </div>
          {"\n  "}
        </div>
        {"\n"}
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap">
          {"\n  "}
          <div className="over v">{"How it runs"}</div>
          <h2 className="h2" style={{ margin: "10px 0 32px" }}>
            {"From message to done"}
          </h2>
          {"\n  "}
          <div className="flow">
            <div className="fn ">
              <div className="k">{"Trigger"}</div>
              <b>{"A customer asks a question"}</b>
              {"Telegram or X DM"}
            </div>
            <div className="fn ">
              <div className="k">{"Assistant"}</div>
              <b>{"Finds the answer"}</b>
              {"your FAQ + order sheet"}
            </div>
            <div className="fn ">
              <div className="k">{"Action"}</div>
              <b>{"Replies instantly"}</b>
              {"with order status or policy"}
            </div>
            <div className="fn gate">
              <div className="k">{"Your OK"}</div>
              <b>{"Refunds go to a human"}</b>
              {"alert by email"}
            </div>
            <div className="fn ">
              <div className="k">{"Result"}</div>
              <b>{"Question closed"}</b>
              {"or handed off with context"}
            </div>
          </div>
          {"\n"}
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap split">
          {"\n  "}
          <div>
            <div className="over v">{"Safe by default"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"What it’s allowed to do"}
            </h2>
            <p className="lead" style={{ marginTop: "14px" }}>
              {
                "These permissions come pre-set. Change any of them before you go live, or later in one click."
              }
            </p>
            {"\n  "}
            <div
              style={{
                display: "flex",
                gap: "8px",
                flexWrap: "wrap",
                marginTop: "22px",
              }}
            >
              <span className="badge b-ok">{"Allowed"}</span>
              <span className="badge b-ro">{"Read-only"}</span>
              <span className="badge b-ask">{"Ask me first"}</span>
              <span className="badge b-no">{"Not allowed"}</span>
            </div>
            {"\n  "}
            <a
              className="more"
              style={{ marginTop: "22px" }}
              href={s.siteHref("/security")}
            >
              {"Learn how limits work "}
              <svg className="i" viewBox="0 0 24 24">
                <path d="M5 12h14M12 5l7 7-7 7"></path>
              </svg>
            </a>
          </div>
          {"\n  "}
          <div className="perm-card">
            <div className="pc-h">
              <div>
                <b>{"24-hour help desk"}</b>
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
                <small>{"answer customers"}</small>
              </div>
              <span className="badge b-ok">{"Allowed"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#27272A" }}>
                {"X"}
              </span>
              <div>
                {"X DMs"}
                <small>{"answer customers"}</small>
              </div>
              <span className="badge b-ok">{"Allowed"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#0F9D58" }}>
                {"SH"}
              </span>
              <div>
                {"Google Sheets"}
                <small>{"look up orders"}</small>
              </div>
              <span className="badge b-ro">{"Read-only"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#EA4335" }}>
                {"GM"}
              </span>
              <div>
                {"Gmail"}
                <small>{"alert your team"}</small>
              </div>
              <span className="badge b-ok">{"Allowed"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#52525B" }}>
                {"$"}
              </span>
              <div>
                {"Payments"}
                <small>{"issue refunds"}</small>
              </div>
              <span className="badge b-no">{"Not allowed"}</span>
            </div>
            <div className="small" style={{ marginTop: "12px" }}>
              {
                "Under the hood: a NyxID Agent Key with operation scopes and an approval mode."
              }
            </div>
          </div>
          {"\n"}
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap split">
          {"\n  "}
          <div>
            <div className="over v">{"Setup"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"You fill in a few details"}
            </h2>
            <p className="lead" style={{ marginTop: "14px" }}>
              {
                "Answer in plain words. You can change anything later by telling NyxBot in chat."
              }
            </p>
          </div>
          {"\n  "}
          <div className="card">
            <ul className="checks">
              <li>
                <span className="ck">{"1"}</span>
                <div>
                  <b>{"Your FAQ or help doc"}</b>
                </div>
              </li>
              <li>
                <span className="ck">{"2"}</span>
                <div>
                  <b>{"Where orders live (sheet or store)"}</b>
                </div>
              </li>
              <li>
                <span className="ck">{"3"}</span>
                <div>
                  <b>{"Who gets escalations"}</b>
                </div>
              </li>
              <li>
                <span className="ck">{"4"}</span>
                <div>
                  <b>{"Business hours for human follow-up"}</b>
                </div>
              </li>
            </ul>
          </div>
          {"\n"}
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap">
          <div className="over v">{"Pricing"}</div>
          <h2 className="h2" style={{ margin: "10px 0 28px" }}>
            {"What it costs"}
          </h2>
          {"\n  "}
          <div className="grid g2">
            {"\n    "}
            <div className="card">
              <span className="badge b-ok">{"● Usage"}</span>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"Platform prices"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {
                  "You pay what each service charges for this solution: Telegram, X DMs, Google Sheets, Gmail and the AI model, with no markup. Network fees are on us during early access."
                }
              </p>
              <a
                className="more"
                style={{ marginTop: "12px" }}
                href={s.siteHref("/pricing")}
              >
                {"How pricing works "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </a>
            </div>
            {"\n    "}
            <div className="card">
              <span className="badge b-m">{"Business & enterprise"}</span>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"Custom bundle"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {
                  "Running 24-hour help desk at scale? We’ll build a bundle around your volume, deployment and support needs."
                }
              </p>
              <a
                className="more"
                style={{ marginTop: "12px" }}
                href={s.siteHref("/contact?topic=bundle")}
              >
                {"Talk to us "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
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
          <div className="over v">{"Questions"}</div>
          <h2 className="h2" style={{ margin: "10px 0 18px" }}>
            {"About this solution"}
          </h2>
          <details>
            <summary>{"Will it make things up?"}</summary>
            <p>
              {
                "It answers from the documents and data you connect. When it does not know, it says so and hands off."
              }
            </p>
          </details>
          <details>
            <summary>{"Can it handle many chats at once?"}</summary>
            <p>{"Yes. Every conversation runs in parallel."}</p>
          </details>
          <details>
            <summary>{"Can it issue refunds?"}</summary>
            <p>
              {
                "Not by default. Refunds are “Not allowed” in this template; you can switch them to “Ask me first”."
              }
            </p>
          </details>
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap">
          <div className="sechead">
            <h2 className="h2">{"Related solutions"}</h2>
            <a className="btn" href={s.siteHref("/solutions")}>
              {"All solutions "}
              <svg className="i" viewBox="0 0 24 24">
                <path d="M5 12h14M12 5l7 7-7 7"></path>
              </svg>
            </a>
          </div>
          <div className="grid g3">
            <VisualLink
              className="card sol feat"
              href={s.siteHref("/solutions/restaurant-manager")}
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
              href={s.siteHref("/solutions/appointment-booker")}
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
          </div>
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap">
          <div className="band">
            <h2 className="h2">{"Try 24-hour help desk"}</h2>
            <p style={{ opacity: ".88", marginTop: "12px", fontSize: "17px" }}>
              {"Set it up in minutes. Turn it off any time."}
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
