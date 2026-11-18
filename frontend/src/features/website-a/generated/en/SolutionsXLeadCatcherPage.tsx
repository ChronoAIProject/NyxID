// Generated from approved A, SHA-256 5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33.
// Run: node scripts/website-a/generate.mjs <baseline.html>
import { SiteLink } from "../../site-link";
import { VisualLink } from "../../visuals/elements";
import { SolutionActivation } from "../../solution-activation";
import { useWebsite } from "../../website-context";
export default function SolutionsXLeadCatcherPage() {
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
              <span>{"Sales & marketing"}</span>
            </div>
            {"\n      "}
            <div className="ic lg g">
              <svg className="i" viewBox="0 0 24 24">
                <path d="m3 11 18-5v12L3 14v-3z"></path>
                <path d="M11.6 16.8a3 3 0 1 1-5.8-1.6"></path>
              </svg>
            </div>
            {"\n      "}
            <h1
              className="h1"
              style={{ fontSize: "var(--fs-h1)", marginTop: "20px" }}
            >
              {"X lead catcher"}
            </h1>
            {"\n      "}
            <p className="lead" style={{ margin: "16px 0 0" }}>
              {
                "Replies to product questions in your X DMs, qualifies the lead and adds it to your sheet."
              }
            </p>
            {"\n      "}
            <p
              style={{ marginTop: "14px", fontWeight: "600", fontSize: "17px" }}
            >
              {"No DM left on read."}
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
              <span className="badge b-ok">{"● X"}</span>
              <span className="badge b-m">{"X DMs"}</span>
              <span className="badge b-m">{"Google Sheets"}</span>
              <span className="badge b-m">{"X (Twitter)"}</span>
            </div>
            {"\n    "}
          </div>
          {"\n    "}
          <div className="phone">
            <div className="ph-h">
              <div className="av">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="m3 11 18-5v12L3 14v-3z"></path>
                  <path d="M11.6 16.8a3 3 0 1 1-5.8-1.6"></path>
                </svg>
              </div>
              <div>
                <b style={{ fontSize: "14px" }}>{"X lead catcher"}</b>
                <div className="small" style={{ fontSize: "11.5px" }}>
                  {"X · assistant"}
                </div>
              </div>
            </div>
            <div className="msgs">
              <div className="m in">
                {"Hey, does your app work for a team of 20?"}
              </div>
              <div className="m out">
                {
                  "It does! The Teams plan starts at 10 seats. Want me to set up a quick demo?"
                }
              </div>
              <div className="m in">{"Sure, next week"}</div>
              <div className="m tool">{"✓ lead added · demo requested"}</div>
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
              <b>{"New DM"}</b>
              {"X"}
            </div>
            <div className="fn ">
              <div className="k">{"Assistant"}</div>
              <b>{"Answers & qualifies"}</b>
              {"size, need, timing"}
            </div>
            <div className="fn ">
              <div className="k">{"Action"}</div>
              <b>{"Logs the lead"}</b>
              {"Google Sheets"}
            </div>
            <div className="fn gate">
              <div className="k">{"Your OK"}</div>
              <b>{"Not needed"}</b>
              {"it can’t post publicly"}
            </div>
            <div className="fn ">
              <div className="k">{"Result"}</div>
              <b>{"Warm lead in your sheet"}</b>
              {"ready for follow-up"}
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
                <b>{"X lead catcher"}</b>
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
              <span className="gl" style={{ background: "#27272A" }}>
                {"X"}
              </span>
              <div>
                {"X DMs"}
                <small>{"reply to messages"}</small>
              </div>
              <span className="badge b-ok">{"Allowed"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#0F9D58" }}>
                {"SH"}
              </span>
              <div>
                {"Google Sheets"}
                <small>{"add leads"}</small>
              </div>
              <span className="badge b-ok">{"Allowed"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#27272A" }}>
                {"X"}
              </span>
              <div>
                {"X (Twitter)"}
                <small>{"post publicly"}</small>
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
                  <b>{"Your product facts & pricing"}</b>
                </div>
              </li>
              <li>
                <span className="ck">{"2"}</span>
                <div>
                  <b>{"Which questions qualify a lead"}</b>
                </div>
              </li>
              <li>
                <span className="ck">{"3"}</span>
                <div>
                  <b>{"Your lead sheet"}</b>
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
                  "You pay what each service charges for this solution: X DMs, Google Sheets and the AI model, with no markup. Network fees are on us during early access."
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
                  "Running X lead catcher at scale? We’ll build a bundle around your volume, deployment and support needs."
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
            <summary>{"Can it tweet for me?"}</summary>
            <p>{"No. Public posting is off in this template."}</p>
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
          </div>
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap">
          <div className="band">
            <h2 className="h2">{"Try X lead catcher"}</h2>
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
