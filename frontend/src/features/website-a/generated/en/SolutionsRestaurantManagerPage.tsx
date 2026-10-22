// Generated from approved A, SHA-256 5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33.
// Run: node scripts/website-a/generate.mjs <baseline.html>
import { SiteLink } from "../../site-link";
import { VisualLink } from "../../visuals/elements";
import { SolutionActivation } from "../../solution-activation";
import { useWebsite } from "../../website-context";
export default function SolutionsRestaurantManagerPage() {
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
              <SiteLink href={s.siteHref("/solutions")}>{"Solutions"}</SiteLink>
              <span>{"/"}</span>
              <span>{"Hospitality"}</span>
            </div>
            {"\n      "}
            <div className="ic lg g">
              <svg className="i" viewBox="0 0 24 24">
                <path d="M3 2v7c0 1.1.9 2 2 2h4a2 2 0 0 0 2-2V2"></path>
                <path d="M7 2v20"></path>
                <path d="M21 15V2a5 5 0 0 0-5 5v6c0 1.1.9 2 2 2h3Zm0 0v7"></path>
              </svg>
            </div>
            {"\n      "}
            <h1
              className="h1"
              style={{ fontSize: "var(--fs-h1)", marginTop: "20px" }}
            >
              {"Restaurant manager"}
            </h1>
            {"\n      "}
            <p className="lead" style={{ margin: "16px 0 0" }}>
              {
                "Takes table bookings in Telegram, checks your calendar, notes allergies and sends reminders."
              }
            </p>
            {"\n      "}
            <p
              style={{ marginTop: "14px", fontWeight: "600", fontSize: "17px" }}
            >
              {"Never miss a late-night booking again."}
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
              <SiteLink className="btn lg" href={s.siteHref("/how-it-works")}>
                {"How it works"}
              </SiteLink>
            </div>
            {"\n      "}
            <div className="pills" style={{ marginTop: "20px" }}>
              <span className="badge b-ok">{"● Telegram"}</span>
              <span className="badge b-m">{"Telegram"}</span>
              <span className="badge b-m">{"Google Calendar"}</span>
              <span className="badge b-m">{"Google Sheets"}</span>
              <span className="badge b-m">{"Payments"}</span>
            </div>
            {"\n    "}
          </div>
          {"\n    "}
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
                <b style={{ fontSize: "14px" }}>{"Restaurant manager"}</b>
                <div className="small" style={{ fontSize: "11.5px" }}>
                  {"Telegram · assistant"}
                </div>
              </div>
            </div>
            <div className="msgs">
              <div className="m sys">{"Sat 11:42 PM · restaurant closed"}</div>
              <div className="m in">
                {
                  "Hi, Sarah here! Table for 4 tomorrow at 7:30? One of us is vegan."
                }
              </div>
              <div className="m tool">{"✓ checked calendar · 7:30 free"}</div>
              <div className="m tool">{"✓ booking created · note added"}</div>
              <div className="m out">
                {
                  "Booked! Table for 4, Sun 7:30 PM under Sarah. I’ve noted vegan, so the kitchen knows."
                }
              </div>
              <div className="m in">{"Perfect, thank you!"}</div>
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
              <b>{"A guest messages your bot"}</b>
              {"Telegram, any hour"}
            </div>
            <div className="fn ">
              <div className="k">{"Assistant"}</div>
              <b>{"Understands the request"}</b>
              {"date, time, party size, notes"}
            </div>
            <div className="fn ">
              <div className="k">{"Action"}</div>
              <b>{"Checks and books"}</b>
              {"Google Calendar + guest sheet"}
            </div>
            <div className="fn gate">
              <div className="k">{"Your OK"}</div>
              <b>{"Only for cancellations"}</b>
              {"one tap on your phone"}
            </div>
            <div className="fn ">
              <div className="k">{"Result"}</div>
              <b>{"Guest confirmed"}</b>
              {"reminder sent the day before"}
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
            <SiteLink
              className="more"
              style={{ marginTop: "22px" }}
              href={s.siteHref("/security")}
            >
              {"Learn how limits work "}
              <svg className="i" viewBox="0 0 24 24">
                <path d="M5 12h14M12 5l7 7-7 7"></path>
              </svg>
            </SiteLink>
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
                  <b>{"Your opening hours and table sizes"}</b>
                </div>
              </li>
              <li>
                <span className="ck">{"2"}</span>
                <div>
                  <b>{"Which calendar holds bookings"}</b>
                </div>
              </li>
              <li>
                <span className="ck">{"3"}</span>
                <div>
                  <b>{"Your menu or allergy notes (optional)"}</b>
                </div>
              </li>
              <li>
                <span className="ck">{"4"}</span>
                <div>
                  <b>{"Tone and language for replies"}</b>
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
                  "You pay what each service charges for this solution: Telegram, Google Calendar, Google Sheets and the AI model, with no markup. Network fees are on us during early access."
                }
              </p>
              <SiteLink
                className="more"
                style={{ marginTop: "12px" }}
                href={s.siteHref("/pricing")}
              >
                {"How pricing works "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </SiteLink>
            </div>
            {"\n    "}
            <div className="card">
              <span className="badge b-m">{"Business & enterprise"}</span>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"Custom bundle"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {
                  "Running Restaurant manager at scale? We’ll build a bundle around your volume, deployment and support needs."
                }
              </p>
              <SiteLink
                className="more"
                style={{ marginTop: "12px" }}
                href={s.siteHref("/contact?topic=bundle")}
              >
                {"Talk to us "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </SiteLink>
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
            <summary>{"What if we are fully booked?"}</summary>
            <p>
              {
                "It offers the nearest free times and can add the guest to a waitlist in your sheet."
              }
            </p>
          </details>
          <details>
            <summary>{"Can guests cancel?"}</summary>
            <p>
              {
                "Yes, but by default a cancellation comes to you first for a one-tap OK. You can change that."
              }
            </p>
          </details>
          <details>
            <summary>{"Does it work with my booking system?"}</summary>
            <p>
              {
                "It works with Google Calendar and Sheets out of the box. Systems with an API can be connected, and systems without one can be used through a connected computer."
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
            <SiteLink className="btn" href={s.siteHref("/solutions")}>
              {"All solutions "}
              <svg className="i" viewBox="0 0 24 24">
                <path d="M5 12h14M12 5l7 7-7 7"></path>
              </svg>
            </SiteLink>
          </div>
          <div className="grid g3">
            <VisualLink
              className="card sol "
              href={s.siteHref("/solutions/help-desk")}
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
              href={s.siteHref("/solutions/stock-analyst")}
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
          </div>
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap">
          <div className="band">
            <h2 className="h2">{"Try Restaurant manager"}</h2>
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
