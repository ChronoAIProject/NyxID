// Generated from approved A, SHA-256 5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33.
// Run: node scripts/website-a/generate.mjs <baseline.html>
import { useWebsite } from "../../website-context";
export default function SecurityPage() {
  const s = useWebsite();
  void s;
  return (
    <>
      {"\n"}
      <section className="hero glowbg" style={{ position: "relative" }}>
        {"\n  "}
        <div className="wrap">
          <div className="ic lg" style={{ margin: "0 auto" }}>
            <svg className="i" viewBox="0 0 24 24">
              <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"></path>
            </svg>
          </div>
          <h1
            className="h1"
            style={{ marginTop: "22px", fontSize: "var(--fs-h1)" }}
          >
            {"Your passwords never"}
            <br />
            <span className="grad">{"reach the AI."}</span>
          </h1>
          <p className="lead">
            {
              "Logins stay encrypted inside NyxID. Each assistant gets a scoped key for only the actions you allow, risky steps wait for your OK, and every call is logged."
            }
          </p>
        </div>
        {"\n"}
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap">
          <div className="grid g3">
            {"\n  "}
            <div className="card">
              <div className="ic">
                <svg className="i" viewBox="0 0 24 24">
                  <circle cx="7.5" cy="15.5" r="5.5"></circle>
                  <path d="m21 2-9.6 9.6M15.5 7.5l3 3L22 7l-3-3"></path>
                </svg>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"Limited keys"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {"Each assistant gets only the actions its job needs."}
              </p>
            </div>
            <div className="card">
              <div className="ic">
                <svg className="i" viewBox="0 0 24 24">
                  <rect x="3" y="11" width="18" height="11" rx="2"></rect>
                  <path d="M7 11V7a5 5 0 0 1 10 0v4"></path>
                </svg>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"Passwords in a vault"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {"Logins are encrypted and never shown to the AI."}
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
                {"Human approval"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {"Risky actions wait for your yes."}
              </p>
            </div>
            <div className="card">
              <div className="ic">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M8 6h13M8 12h13M8 18h13M3 6h.01M3 12h.01M3 18h.01"></path>
                </svg>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"Tamper-evident log"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {"Every action recorded in a hash-chained log."}
              </p>
            </div>
            <div className="card">
              <div className="ic">
                <svg className="i" viewBox="0 0 24 24">
                  <rect x="2" y="3" width="20" height="14" rx="2"></rect>
                  <path d="M8 21h8M12 17v4"></path>
                </svg>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"Computer access, opt-in"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {
                  "Browser, files, desktop and shell are off until you switch each one on."
                }
              </p>
            </div>
            <div className="card">
              <div className="ic">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M18.36 6.64a9 9 0 1 1-12.73 0"></path>
                  <path d="M12 2v10"></path>
                </svg>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"Instant off switch"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {"Pause an assistant or revoke a key in one click."}
              </p>
            </div>
            {"\n"}
          </div>
        </div>
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap split">
          {"\n  "}
          <div>
            <div className="over v">{"1 · Limited keys"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"Only what the job needs"}
            </h2>
            <p className="lead" style={{ marginTop: "14px" }}>
              {
                "Most AI tools ask for full access to an account. NyxID breaks access down action by action: “see free times” and “create a booking” are separate from “cancel a booking” or “send an email”."
              }
            </p>
            {"\n  "}
            <ul className="checks" style={{ marginTop: "24px" }}>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M20 6 9 17l-5-5"></path>
                  </svg>
                </span>
                <div>
                  <b>{"One key per assistant"}</b>
                  <span>
                    {"So one assistant’s access never leaks into another’s."}
                  </span>
                </div>
              </li>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M20 6 9 17l-5-5"></path>
                  </svg>
                </span>
                <div>
                  <b>{"Limits on the details"}</b>
                  <span>{"For example: “only call numbers on my list”."}</span>
                </div>
              </li>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M20 6 9 17l-5-5"></path>
                  </svg>
                </span>
                <div>
                  <b>{"Speed limits"}</b>
                  <span>{"Per-assistant rate limits stop runaway loops."}</span>
                </div>
              </li>
            </ul>
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
          </div>
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap split rev">
          {"\n  "}
          <div>
            <div className="over v">{"2 · Password vault"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"Your logins never reach the AI"}
            </h2>
            <p className="lead" style={{ marginTop: "14px" }}>
              {
                "You sign in on the service’s own page. NyxID stores the result with envelope encryption and adds it to each request at the last moment, so it never appears in a prompt, a chat or an AI provider’s logs."
              }
            </p>
          </div>
          {"\n  "}
          <div className="card" style={{ padding: "28px" }}>
            {"\n    "}
            <div
              className="grid vault"
              style={{
                gridTemplateColumns: "1fr auto 1fr auto 1fr",
                alignItems: "center",
                gap: "10px",
                textAlign: "center",
                fontSize: "13px",
              }}
            >
              {"\n      "}
              <div>
                <div className="ic" style={{ margin: "0 auto" }}>
                  <svg className="i" viewBox="0 0 24 24">
                    <circle cx="12" cy="8" r="4"></circle>
                    <path d="M4 21v-1a6 6 0 0 1 6-6h4a6 6 0 0 1 6 6v1"></path>
                  </svg>
                </div>
                <b style={{ display: "block", marginTop: "8px" }}>{"You"}</b>
                <span className="small">{"sign in to Google"}</span>
              </div>
              <span className="small">{"→"}</span>
              {"\n      "}
              <div>
                <div className="ic g lg" style={{ margin: "0 auto" }}>
                  <svg className="i" viewBox="0 0 24 24">
                    <rect x="3" y="11" width="18" height="11" rx="2"></rect>
                    <path d="M7 11V7a5 5 0 0 1 10 0v4"></path>
                  </svg>
                </div>
                <b style={{ display: "block", marginTop: "8px" }}>
                  {"NyxID vault"}
                </b>
                <span className="small">{"encrypted, AES-256"}</span>
              </div>
              <span className="small">{"→"}</span>
              {"\n      "}
              <div>
                <div className="ic" style={{ margin: "0 auto" }}>
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M12 3l1.9 5.8L20 11l-6.1 2.2L12 19l-1.9-5.8L4 11l6.1-2.2z"></path>
                  </svg>
                </div>
                <b style={{ display: "block", marginTop: "8px" }}>
                  {"Your assistant"}
                </b>
                <span className="small">{"gets a limited key"}</span>
              </div>
              {"\n    "}
            </div>
            {"\n    "}
            <div className="ph" style={{ marginTop: "22px" }}>
              {"The AI sees: "}
              <code style={{ margin: "0 6px" }}>{"nyxid_ag_••••"}</code>
              {" · never your password"}
            </div>
            {"\n  "}
          </div>
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap">
          {"\n  "}
          <div style={{ maxWidth: "620px" }}>
            <div className="over v">{"3 · Human approval"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"You decide how much it asks"}
            </h2>
            <p className="lead" style={{ marginTop: "14px" }}>
              {
                "Choose an access mode for your assistants. Set it once, or differently for each assistant."
              }
            </p>
          </div>
          {"\n  "}
          <div className="grid g3" style={{ marginTop: "32px" }}>
            {"\n    "}
            <div className="card">
              <span className="badge b-m">{"Autopilot"}</span>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"Just do it"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {"No approval cards. Best for read-only jobs like news briefs."}
              </p>
            </div>
            {"\n    "}
            <div className="card feat">
              <span className="badge b-v">{"NyxMode · default"}</span>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"Ask for the big stuff"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {
                  "Asks before sensitive or destructive actions, such as deleting, cancelling or paying."
                }
              </p>
            </div>
            {"\n    "}
            <div className="card">
              <span className="badge b-ask">{"Ask every time"}</span>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"Check everything"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {"Asks before any change. Great while you build trust."}
              </p>
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
            <div className="over v">{"4 · Computer access"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"Your machine, your switches"}
            </h2>
            <p className="lead" style={{ marginTop: "14px" }}>
              {
                "When NyxBot uses a computer, every capability is off by default and controlled on the machine itself. Only you, the owner, can use it; customers chatting with your bots never can."
              }
            </p>
            {"\n  "}
            <ul className="checks" style={{ marginTop: "24px" }}>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M20 6 9 17l-5-5"></path>
                  </svg>
                </span>
                <div>
                  <b>{"Signed requests only"}</b>
                  <span>
                    {"The computer only accepts requests signed for it."}
                  </span>
                </div>
              </li>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M20 6 9 17l-5-5"></path>
                  </svg>
                </span>
                <div>
                  <b>{"Hidden saved logins"}</b>
                  <span>
                    {"Filled into the exact website, never shown to the AI."}
                  </span>
                </div>
              </li>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M20 6 9 17l-5-5"></path>
                  </svg>
                </span>
                <div>
                  <b>{"Stronger isolation available"}</b>
                  <span>
                    {
                      "Use the NyxID container or a hosted VM to keep work apart from your personal files."
                    }
                  </span>
                </div>
              </li>
            </ul>
          </div>
          {"\n  "}
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
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap split rev">
          {"\n  "}
          <div>
            <div className="over v">{"5 · Activity log"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"A record you can trust"}
            </h2>
            <p className="lead" style={{ marginTop: "14px" }}>
              {
                "Every action is written to an audit log where each entry is chained to the one before it, so edits or deletions show up. You always know which assistant did what, and when."
              }
            </p>
          </div>
          {"\n  "}
          <div className="card log">
            {"\n    "}
            <div className="r">
              <span className="tm">{"23:42:08"}</span>
              <span>{"Restaurant manager · calendar.create_event"}</span>
              <span className="badge b-ok">{"allowed"}</span>
            </div>
            <div className="r">
              <span className="tm">{"19:40:31"}</span>
              <span>{"Restaurant manager · calendar.delete_event"}</span>
              <span className="badge b-ask">{"approved by you"}</span>
            </div>
            <div className="r">
              <span className="tm">{"14:10:02"}</span>
              <span>{"Back-office clerk · machine.browser"}</span>
              <span className="badge b-ask">{"approved by you"}</span>
            </div>
            <div className="r">
              <span className="tm">{"15:05:12"}</span>
              <span>{"Callback queue · twilio.create_call"}</span>
              <span className="badge b-ok">{"allowed"}</span>
            </div>
            <div className="r">
              <span className="tm">{"15:05:44"}</span>
              <span>{"Callback queue · twilio.create_call (+44…)"}</span>
              <span className="badge b-no">{"blocked: not on list"}</span>
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
            <div className="over v">{"6 · Off switch"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"Stop anything, instantly"}
            </h2>
            <p className="lead" style={{ marginTop: "14px" }}>
              {
                "Disable an assistant, revoke a key or disconnect a service in one click. A running task stops, and the AI loses access immediately."
              }
            </p>
          </div>
          {"\n  "}
          <div className="card mock" style={{ display: "grid", gap: "12px" }}>
            <div
              style={{
                display: "flex",
                justifyContent: "space-between",
                alignItems: "center",
                padding: "6px 0",
              }}
            >
              <b style={{ fontSize: "14.5px" }}>{"Restaurant manager"}</b>
              <span
                style={{ display: "flex", gap: "10px", alignItems: "center" }}
              >
                <span className="badge b-ok">{"Active"}</span>
                <button className="btn sm" type="button">
                  {"Disable"}
                </button>
              </span>
            </div>
            <div
              style={{
                display: "flex",
                justifyContent: "space-between",
                alignItems: "center",
                padding: "6px 0",
              }}
            >
              <b style={{ fontSize: "14.5px" }}>{"Back-office clerk"}</b>
              <span
                style={{ display: "flex", gap: "10px", alignItems: "center" }}
              >
                <span className="badge b-ok">{"Active"}</span>
                <button className="btn sm" type="button">
                  {"Disable"}
                </button>
              </span>
            </div>
            <div
              style={{
                display: "flex",
                justifyContent: "space-between",
                alignItems: "center",
                padding: "6px 0",
              }}
            >
              <b style={{ fontSize: "14.5px" }}>{"News aggregator"}</b>
              <span
                style={{ display: "flex", gap: "10px", alignItems: "center" }}
              >
                <span className="badge b-m">{"Paused"}</span>
                <button className="btn sm" type="button">
                  {"Enable"}
                </button>
              </span>
            </div>
          </div>
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap">
          {"\n  "}
          <div className="over v">{"For your IT team"}</div>
          <h2 className="h2" style={{ margin: "10px 0 32px" }}>
            {"Under the hood"}
          </h2>
          {"\n  "}
          <div className="grid g3">
            <div className="card">
              <b style={{ fontSize: "15px" }}>{"Encryption"}</b>
              <p
                className="small"
                style={{ marginTop: "8px", fontSize: "14px" }}
              >
                {
                  "AES-256 envelope encryption; AWS KMS and GCP Cloud KMS support; key material held in zeroizing memory."
                }
              </p>
            </div>
            <div className="card">
              <b style={{ fontSize: "15px" }}>{"Identity"}</b>
              <p
                className="small"
                style={{ marginTop: "8px", fontSize: "14px" }}
              >
                {
                  "OAuth 2.0 / OpenID Connect with PKCE, MFA, SSO, short-lived tokens."
                }
              </p>
            </div>
            <div className="card">
              <b style={{ fontSize: "15px" }}>{"Agent isolation"}</b>
              <p
                className="small"
                style={{ marginTop: "8px", fontSize: "14px" }}
              >
                {
                  "Per-agent keys with service allowlists, operation scopes, input limits and rate limits."
                }
              </p>
            </div>
            <div className="card">
              <b style={{ fontSize: "15px" }}>{"Audit"}</b>
              <p
                className="small"
                style={{ marginTop: "8px", fontSize: "14px" }}
              >
                {
                  "HMAC hash-chained audit log, verifiable from the admin Integrity page."
                }
              </p>
            </div>
            <div className="card">
              <b style={{ fontSize: "15px" }}>{"Machine nodes"}</b>
              <p
                className="small"
                style={{ marginTop: "8px", fontSize: "14px" }}
              >
                {
                  "Locally authoritative shell/files/computer switches, mandatory signed requests, owner-only use, human live desktop."
                }
              </p>
            </div>
            <div className="card">
              <b style={{ fontSize: "15px" }}>{"On-premise option"}</b>
              <p
                className="small"
                style={{ marginTop: "8px", fontSize: "14px" }}
              >
                {
                  "Credential nodes keep secrets on your own machines; NyxID routes requests through them."
                }
              </p>
            </div>
          </div>
          {"\n  "}
          <div
            className="card"
            style={{
              marginTop: "16px",
              display: "flex",
              justifyContent: "space-between",
              alignItems: "center",
              gap: "20px",
              flexWrap: "wrap",
            }}
          >
            <div style={{ maxWidth: "640px" }}>
              <b style={{ fontSize: "var(--fs-h3)" }}>
                {"Open source, Apache-2.0"}
              </b>
              <p
                className="sub"
                style={{ fontSize: "14.5px", marginTop: "6px" }}
              >
                {
                  "Every claim on this page is checkable. Read the code, run it yourself with Docker, or have your own AI audit how keys are stored, decrypted and scoped."
                }
              </p>
            </div>
            <a
              className="btn"
              href="https://github.com/ChronoAIProject/NyxID"
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
                <path d="M12 .5C5.65.5.5 5.65.5 12a11.5 11.5 0 0 0 7.86 10.92c.58.1.79-.25.79-.56v-2c-3.2.7-3.88-1.37-3.88-1.37-.52-1.33-1.28-1.69-1.28-1.69-1.04-.71.08-.7.08-.7 1.16.08 1.77 1.19 1.77 1.19 1.03 1.76 2.7 1.25 3.36.96.1-.75.4-1.25.73-1.54-2.56-.29-5.25-1.28-5.25-5.68 0-1.26.45-2.28 1.19-3.09-.12-.29-.52-1.46.11-3.05 0 0 .97-.31 3.17 1.18a11 11 0 0 1 5.77 0c2.2-1.49 3.17-1.18 3.17-1.18.63 1.59.23 2.76.11 3.05.74.81 1.19 1.83 1.19 3.09 0 4.41-2.69 5.38-5.26 5.67.41.36.78 1.06.78 2.14v3.17c0 .31.21.67.8.56A11.5 11.5 0 0 0 23.5 12C23.5 5.65 18.35.5 12 .5Z"></path>
              </svg>
              {" View on GitHub"}
            </a>
          </div>
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap" style={{ maxWidth: "860px" }}>
          <div className="over v">{"FAQ"}</div>
          <h2 className="h2" style={{ margin: "10px 0 18px" }}>
            {"Security questions"}
          </h2>
          <details>
            <summary>{"Can NyxID staff see my passwords?"}</summary>
            <p>
              {
                "Logins are stored encrypted and are only decrypted to make an allowed request. They are never shown in the dashboard or API responses."
              }
            </p>
          </details>
          <details>
            <summary>
              {"What if an AI is tricked by a customer message?"}
            </summary>
            <p>
              {
                "It can still only do what its key allows. Customers can never reach your computer, anything sensitive needs your approval, and blocked attempts appear in your log."
              }
            </p>
          </details>
          <details>
            <summary>{"Is giving AI my computer risky?"}</summary>
            <p>
              {
                "Any computer access carries risk, which is why each capability is off by default and every change needs a switch you flip. For the strongest separation, use the container or a hosted VM instead of your daily machine."
              }
            </p>
          </details>
          <details>
            <summary>{"Where is my data stored?"}</summary>
            <p>
              {
                "Contact the team to confirm current hosting and certification details."
              }
            </p>
          </details>
          <details>
            <summary>{"Do you have certifications?"}</summary>
            <p>
              {
                "Contact the team to confirm current hosting and certification details."
              }
            </p>
          </details>
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap">
          <div className="band">
            <h2 className="h2">{"Automation you can trust."}</h2>
            <p style={{ opacity: ".88", marginTop: "12px", fontSize: "17px" }}>
              {"Start with read-only solutions and add more as you go."}
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
