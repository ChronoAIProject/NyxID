// Generated from approved A, SHA-256 5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33.
// Run: node scripts/website-a/generate.mjs <baseline.html>
import { useWebsite } from "../../website-context";
export default function SolutionsBackOfficeClerkPage() {
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
              <span>{"Operations"}</span>
            </div>
            {"\n      "}
            <div className="ic lg g">
              <svg className="i" viewBox="0 0 24 24">
                <rect x="2" y="3" width="20" height="14" rx="2"></rect>
                <path d="M8 21h8M12 17v4"></path>
              </svg>
            </div>
            {"\n      "}
            <h1
              className="h1"
              style={{ fontSize: "var(--fs-h1)", marginTop: "20px" }}
            >
              {"Back-office clerk"}
            </h1>
            {"\n      "}
            <p className="lead" style={{ margin: "16px 0 0" }}>
              {
                "Logs in to supplier and booking portals that have no API, reorders stock, downloads invoices and files them, on your computer or a hosted VM."
              }
            </p>
            {"\n      "}
            <p
              style={{ marginTop: "14px", fontWeight: "600", fontSize: "17px" }}
            >
              {"If you can click it, your assistant can too."}
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
              <span className="badge b-ok">{"● Telegram"}</span>
              <span className="badge b-ro">{"● Computer"}</span>
              <span className="badge b-m">{"Computer · browser"}</span>
              <span className="badge b-m">{"Computer · files"}</span>
              <span className="badge b-m">{"Google Sheets"}</span>
              <span className="badge b-m">{"Computer · checkout"}</span>
              <span className="badge b-m">{"Computer · shell"}</span>
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
      {"\n"}
      <section className="s">
        <div className="wrap split">
          <div>
            <div className="over v">{"In chat"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"You ask in Telegram. It works on the computer."}
            </h2>
          </div>
          <div className="phone">
            <div className="ph-h">
              <div className="av">
                <svg className="i" viewBox="0 0 24 24">
                  <rect x="2" y="3" width="20" height="14" rx="2"></rect>
                  <path d="M8 21h8M12 17v4"></path>
                </svg>
              </div>
              <div>
                <b style={{ fontSize: "14px" }}>{"Back-office clerk"}</b>
                <div className="small" style={{ fontSize: "11.5px" }}>
                  {"Telegram · assistant"}
                </div>
              </div>
            </div>
            <div className="msgs">
              <div className="m in">
                {"We’re low on tomatoes. Reorder from Fresh Farms."}
              </div>
              <div className="m tool">
                {"✓ opened Fresh Farms portal · Mac mini"}
              </div>
              <div className="m tool">{"✓ signed in with saved login"}</div>
              <div className="m out">
                {"Cart ready: 12 cases of tomatoes, $186. Place the order?"}
              </div>
              <div className="m in">{"Yes"}</div>
              <div className="m tool">{"✓ order placed · invoice saved"}</div>
            </div>
          </div>
        </div>
      </section>
      {"\n"}
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
              <b>{"You ask, or stock runs low"}</b>
              {"Telegram or schedule"}
            </div>
            <div className="fn ">
              <div className="k">{"Assistant"}</div>
              <b>{"Plans the clicks"}</b>
              {"which site, what to order"}
            </div>
            <div className="fn ">
              <div className="k">{"Action"}</div>
              <b>{"Uses the browser"}</b>
              {"on your computer or a hosted VM"}
            </div>
            <div className="fn gate">
              <div className="k">{"Your OK"}</div>
              <b>{"Before submitting"}</b>
              {"order total shown on the card"}
            </div>
            <div className="fn ">
              <div className="k">{"Result"}</div>
              <b>{"Order placed"}</b>
              {"invoice filed, sheet updated"}
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
                <b>{"Back-office clerk"}</b>
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
              <span className="gl" style={{ background: "#4A1FD4" }}>
                {"PC"}
              </span>
              <div>
                {"Computer · browser"}
                <small>{"use portals with saved logins"}</small>
              </div>
              <span className="badge b-ok">{"Allowed"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#4A1FD4" }}>
                {"PC"}
              </span>
              <div>
                {"Computer · files"}
                <small>{"save invoices to a folder"}</small>
              </div>
              <span className="badge b-ok">{"Allowed"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#0F9D58" }}>
                {"SH"}
              </span>
              <div>
                {"Google Sheets"}
                <small>{"update the stock sheet"}</small>
              </div>
              <span className="badge b-ok">{"Allowed"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#4A1FD4" }}>
                {"PC"}
              </span>
              <div>
                {"Computer · checkout"}
                <small>{"submit orders"}</small>
              </div>
              <span className="badge b-ask">{"Ask me first"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#4A1FD4" }}>
                {"PC"}
              </span>
              <div>
                {"Computer · shell"}
                <small>{"run commands"}</small>
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
                  <b>{"Which machine: your computer or a hosted VM"}</b>
                </div>
              </li>
              <li>
                <span className="ck">{"2"}</span>
                <div>
                  <b>{"The websites and saved logins it may use"}</b>
                </div>
              </li>
              <li>
                <span className="ck">{"3"}</span>
                <div>
                  <b>{"Folder for invoices"}</b>
                </div>
              </li>
              <li>
                <span className="ck">{"4"}</span>
                <div>
                  <b>{"Spending limit per order"}</b>
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
                  "You pay what each service charges for this solution: Computer · browser, Computer · files, Google Sheets, Computer · checkout and the AI model, with no markup. Network fees are on us during early access."
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
                  "Running Back-office clerk at scale? We’ll build a bundle around your volume, deployment and support needs."
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
            <summary>{"Does the AI see my portal password?"}</summary>
            <p>
              {
                "No. Saved logins are encrypted and filled directly into the exact website; the AI never sees them."
              }
            </p>
          </details>
          <details>
            <summary>{"Can I watch what it’s doing?"}</summary>
            <p>
              {
                "Yes. Open the live desktop from your browser and take over at any moment."
              }
            </p>
          </details>
          <details>
            <summary>{"Is it safe to use my own computer?"}</summary>
            <p>
              {
                "Machine access is off by default and you switch on browser, files or shell one by one. For stronger isolation, use the NyxID container or a hosted VM."
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
            <a
              className="card sol feat"
              href={s.siteHref("/solutions/restaurant-manager")}
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
            </a>
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
            <a
              className="card sol "
              href={s.siteHref("/solutions/appointment-booker")}
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
            </a>
          </div>
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap">
          <div className="band">
            <h2 className="h2">{"Try Back-office clerk"}</h2>
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
