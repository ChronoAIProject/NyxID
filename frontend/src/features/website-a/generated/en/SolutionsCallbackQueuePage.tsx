// Generated from approved A, SHA-256 5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33.
// Run: node scripts/website-a/generate.mjs <baseline.html>
import { useWebsite } from "../../website-context";
export default function SolutionsCallbackQueuePage() {
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
                <path d="M22 16.92v3a2 2 0 0 1-2.18 2 19.79 19.79 0 0 1-8.63-3.07 19.5 19.5 0 0 1-6-6 19.79 19.79 0 0 1-3.07-8.67A2 2 0 0 1 4.11 2h3a2 2 0 0 1 2 1.72c.13.96.36 1.9.7 2.81a2 2 0 0 1-.45 2.11L8.09 9.91a16 16 0 0 0 6 6l1.27-1.27a2 2 0 0 1 2.11-.45c.91.34 1.85.57 2.81.7A2 2 0 0 1 22 16.92z"></path>
              </svg>
            </div>
            {"\n      "}
            <h1
              className="h1"
              style={{ fontSize: "var(--fs-h1)", marginTop: "20px" }}
            >
              {"Callback queue"}
            </h1>
            {"\n      "}
            <p className="lead" style={{ margin: "16px 0 0" }}>
              {
                "Works through your list of customers to call back, has the conversation with an AI voice, and logs the outcome."
              }
            </p>
            {"\n      "}
            <p
              style={{ marginTop: "14px", fontWeight: "600", fontSize: "17px" }}
            >
              {"Every callback made. Every outcome written down."}
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
              <a className="btn p lg" href={s.authHref("register")}>
                {"Use this solution"}
              </a>
              <a className="btn lg" href={s.siteHref("/how-it-works")}>
                {"How it works"}
              </a>
            </div>
            {"\n      "}
            <div className="pills" style={{ marginTop: "20px" }}>
              <span className="badge b-ro">{"● Phone"}</span>
              <span className="badge b-m">{"Google Sheets"}</span>
              <span className="badge b-m">{"Twilio"}</span>
              <span className="badge b-m">{"ElevenLabs"}</span>
              <span className="badge b-m">{"Discounts & refunds"}</span>
            </div>
            {"\n    "}
          </div>
          {"\n    "}
          <div className="phone">
            <div className="ph-h">
              <div className="av">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M22 16.92v3a2 2 0 0 1-2.18 2 19.79 19.79 0 0 1-8.63-3.07 19.5 19.5 0 0 1-6-6 19.79 19.79 0 0 1-3.07-8.67A2 2 0 0 1 4.11 2h3a2 2 0 0 1 2 1.72c.13.96.36 1.9.7 2.81a2 2 0 0 1-.45 2.11L8.09 9.91a16 16 0 0 0 6 6l1.27-1.27a2 2 0 0 1 2.11-.45c.91.34 1.85.57 2.81.7A2 2 0 0 1 22 16.92z"></path>
                </svg>
              </div>
              <div>
                <b style={{ fontSize: "14px" }}>{"Callback queue"}</b>
                <div className="small" style={{ fontSize: "11.5px" }}>
                  {"Phone · callback queue"}
                </div>
              </div>
            </div>
            <div className="msgs">
              <div className="m sys">{"Weekdays 10:00 AM · scheduled run"}</div>
              <div className="m tool">{"✓ 14 customers to call"}</div>
              <div className="m out">
                {"Calling Maria L. · “asked about weekend slots”"}
              </div>
              <div className="m tool">{"✓ booked Sat 2 PM"}</div>
              <div className="m out">
                {"James K. wants a discount on his quote. Approve 10%?"}
              </div>
              <div className="m in">{"Yes"}</div>
              <div className="m tool">{"✓ approved · calling back"}</div>
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
              <b>{"New rows in “Call back”"}</b>
              {"or every weekday 10 AM"}
            </div>
            <div className="fn ">
              <div className="k">{"Assistant"}</div>
              <b>{"Reads each note"}</b>
              {"prepares what to say"}
            </div>
            <div className="fn ">
              <div className="k">{"Action"}</div>
              <b>{"Places the call"}</b>
              {"Twilio + AI voice"}
            </div>
            <div className="fn gate">
              <div className="k">{"Your OK"}</div>
              <b>{"Discounts or refunds"}</b>
              {"yes/no on your phone"}
            </div>
            <div className="fn ">
              <div className="k">{"Result"}</div>
              <b>{"Outcome logged"}</b>
              {"booked / no answer / later"}
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
                <b>{"Callback queue"}</b>
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
              <span className="gl" style={{ background: "#0F9D58" }}>
                {"SH"}
              </span>
              <div>
                {"Google Sheets"}
                <small>{"read the call list"}</small>
              </div>
              <span className="badge b-ro">{"Read-only"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#0F9D58" }}>
                {"SH"}
              </span>
              <div>
                {"Google Sheets"}
                <small>{"write outcomes"}</small>
              </div>
              <span className="badge b-ok">{"Allowed"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#F22F46" }}>
                {"TW"}
              </span>
              <div>
                {"Twilio"}
                <small>{"call numbers on the list"}</small>
              </div>
              <span className="badge b-ok">{"Allowed"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#18181B" }}>
                {"11"}
              </span>
              <div>
                {"ElevenLabs"}
                <small>{"natural voice"}</small>
              </div>
              <span className="badge b-ok">{"Allowed"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#52525B" }}>
                {"$"}
              </span>
              <div>
                {"Discounts & refunds"}
                <small>{"offer money back"}</small>
              </div>
              <span className="badge b-ask">{"Ask me first"}</span>
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
                  <b>{"Your call-back sheet"}</b>
                </div>
              </li>
              <li>
                <span className="ck">{"2"}</span>
                <div>
                  <b>{"Calling hours"}</b>
                </div>
              </li>
              <li>
                <span className="ck">{"3"}</span>
                <div>
                  <b>{"What the assistant may offer"}</b>
                </div>
              </li>
              <li>
                <span className="ck">{"4"}</span>
                <div>
                  <b>{"A short script or tone guide"}</b>
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
                  "You pay what each service charges for this solution: Google Sheets, Twilio, ElevenLabs, Discounts & refunds and the AI model, with no markup. Network fees are on us during early access."
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
                  "Running Callback queue at scale? We’ll build a bundle around your volume, deployment and support needs."
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
            <summary>{"Does it call through NyxID directly?"}</summary>
            <p>
              {
                "Calls run through your connected Twilio and ElevenLabs accounts; NyxID holds the keys and limits what can be called."
              }
            </p>
          </details>
          <details>
            <summary>{"Can it call any number?"}</summary>
            <p>
              {"No. In this template it may only call numbers on your list."}
            </p>
          </details>
          <details>
            <summary>{"Is it recorded?"}</summary>
            <p>{"Only if you turn on recording in Twilio."}</p>
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
            <a className="card sol " href={s.siteHref("/solutions/help-desk")}>
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
            </a>
          </div>
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap">
          <div className="band">
            <h2 className="h2">{"Try Callback queue"}</h2>
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
