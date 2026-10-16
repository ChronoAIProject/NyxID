// Generated from approved A, SHA-256 5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33.
// Run: node scripts/website-a/generate.mjs <baseline.html>
import { VisualLink } from "../../visuals/elements";
import { useWebsite } from "../../website-context";
export default function PricingPage() {
  const s = useWebsite();
  void s;
  return (
    <>
      {"\n"}
      <section className="hero glowbg" style={{ position: "relative" }}>
        {"\n  "}
        <div className="wrap">
          <div className="over v">{"Pricing"}</div>
          <h1
            className="h1"
            style={{ marginTop: "14px", fontSize: "var(--fs-h1)" }}
          >
            {"Platform prices."}
            <br />
            <span className="grad">{"No markup."}</span>
          </h1>
          <p className="lead">
            {
              "Every model and API call is billed at the provider’s published rate. NyxID adds nothing on top."
            }
          </p>
          <div className="cta">
            <VisualLink
              className="btn p lg"
              href={s.authHref("register")}
              effect="button"
            >
              {"Get started"}
            </VisualLink>
            <a className="btn lg" href={s.siteHref("/contact?topic=bundle")}>
              {"Talk to us"}
            </a>
          </div>
        </div>
        {"\n"}
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap split">
          {"\n  "}
          <div>
            <div className="over v">{"Our commitment"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"Zero markup on "}
              <span className="grad">{"platform charges."}</span>
            </h2>
            <p className="lead" style={{ marginTop: "14px" }}>
              {
                "When your agent calls a model or an API through NyxID, you pay exactly what that provider charges. We never levy our own pricing on top of a platform’s fee."
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
                  <b>{"Pass-through rates"}</b>
                  <span>{"Published provider prices, billed as-is."}</span>
                </div>
              </li>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M2 12s3-7 10-7 10 7 10 7-3 7-10 7-10-7-10-7Z"></path>
                    <circle cx="12" cy="12" r="3"></circle>
                  </svg>
                </span>
                <div>
                  <b>{"Itemised usage"}</b>
                  <span>
                    {"Every charge broken down by agent, service and call."}
                  </span>
                </div>
              </li>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <circle cx="7.5" cy="15.5" r="5.5"></circle>
                    <path d="m21 2-9.6 9.6M15.5 7.5l3 3L22 7l-3-3"></path>
                  </svg>
                </span>
                <div>
                  <b>{"Bring your own keys"}</b>
                  <span>
                    {"Use your own provider accounts and pay them directly."}
                  </span>
                </div>
              </li>
            </ul>
          </div>
          {"\n  "}
          <div className="card" style={{ padding: "28px" }}>
            {"\n    "}
            <div className="over">{"What a call costs"}</div>
            {"\n    "}
            <div style={{ display: "grid", marginTop: "18px" }}>
              {"\n      "}
              <div
                style={{
                  display: "flex",
                  justifyContent: "space-between",
                  alignItems: "center",
                  padding: "12px 0",
                  borderBottom: "1px solid var(--border)",
                }}
              >
                <span>{"Provider charges"}</span>
                <span className="mono" style={{}}>
                  {"$1.00"}
                </span>
              </div>
              <div
                style={{
                  display: "flex",
                  justifyContent: "space-between",
                  alignItems: "center",
                  padding: "12px 0",
                  borderBottom: "1px solid var(--border)",
                }}
              >
                <span>{"NyxID markup"}</span>
                <span className="mono" style={{}}>
                  {"$0.00"}
                </span>
              </div>
              <div
                style={{
                  display: "flex",
                  justifyContent: "space-between",
                  alignItems: "center",
                  padding: "12px 0",
                  borderBottom: "1px solid var(--border)",
                }}
              >
                <span>{"Network fee (early access)"}</span>
                <span className="mono" style={{}}>
                  {"$0.00"}
                </span>
              </div>
              <div
                style={{
                  display: "flex",
                  justifyContent: "space-between",
                  alignItems: "center",
                  padding: "12px 0",
                  borderBottom: "1px solid var(--border)",
                  fontWeight: "700",
                  fontSize: "18px",
                  border: "none",
                }}
              >
                <span>{"You pay"}</span>
                <span className="mono" style={{ color: "var(--ptext)" }}>
                  {"$1.00"}
                </span>
              </div>
              {"\n    "}
            </div>
            {"\n    "}
            <p className="small" style={{ marginTop: "14px" }}>
              {
                "Illustrative. Actual prices are each provider’s published rates."
              }
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
          <div
            className="center"
            style={{ textAlign: "center", maxWidth: "720px" }}
          >
            <div className="over v">{"How pricing works"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"Three parts, no surprises"}
            </h2>
          </div>
          {"\n  "}
          <div className="grid g3" style={{ marginTop: "40px" }}>
            {"\n    "}
            <div className="card">
              <span className="badge b-ok">{"● Usage"}</span>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"Platform charges"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {
                  "Models, APIs, calls and messages at each provider’s rate. Top up a prepaid balance; usage draws it down."
                }
              </p>
            </div>
            {"\n    "}
            <div className="card">
              <span className="badge b-v">{"Early access"}</span>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"Network fees: on us"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {
                  "Running the gateway costs us compute, bandwidth and storage. We absorb it while we learn what it really costs. If we ever introduce a network fee, it will only cover cost, and we’ll aim to announce it before it applies."
                }
              </p>
            </div>
            {"\n    "}
            <div className="card">
              <span className="badge b-m">{"Business & enterprise"}</span>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"Custom bundles"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {
                  "Predictable spend, volume, SSO, on-prem credential nodes and priority support, shaped around your workload."
                }
              </p>
              <a
                className="more"
                style={{ marginTop: "14px" }}
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
          {"\n  "}
          <p
            className="small center"
            style={{ textAlign: "center", marginTop: "20px" }}
          >
            {
              "Always free: the gateway, the vault, scoped keys, approvals and the activity log. Self-hosting the open-source edition is free too."
            }
          </p>
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap" style={{ maxWidth: "860px" }}>
          <div className="over v">{"FAQ"}</div>
          <h2 className="h2" style={{ margin: "10px 0 18px" }}>
            {"Pricing questions"}
          </h2>
          <details>
            <summary>{"How does NyxID make money?"}</summary>
            <p>
              {
                "Through custom bundles for businesses and enterprise: volume, deployment options and support. Not from markup on your usage."
              }
            </p>
          </details>
          <details>
            <summary>{"What is a network fee?"}</summary>
            <p>
              {
                "The cost of running the gateway itself: compute, bandwidth and storage. It’s absorbed during early access. If it’s ever introduced, it will be priced at cost, and we’ll aim to announce it in advance."
              }
            </p>
          </details>
          <details>
            <summary>{"What are credits?"}</summary>
            <p>
              {
                "A prepaid balance. Usage at platform rates draws it down, so there are no surprise bills."
              }
            </p>
          </details>
          <details>
            <summary>{"Can I use my own API keys?"}</summary>
            <p>
              {
                "Yes. With your own provider account, you pay the provider directly for that service."
              }
            </p>
          </details>
          <details>
            <summary>{"Is there a subscription?"}</summary>
            <p>
              {
                "No. Pay for platform usage only, or talk to us about a custom bundle."
              }
            </p>
          </details>
          <details>
            <summary>{"Can I self-host for free?"}</summary>
            <p>
              {
                "Yes. NyxID is open source under Apache-2.0. Run it yourself with Docker, or use the hosted service."
              }
            </p>
          </details>
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap">
          <div className="band">
            <h2 className="h2">{"Pay the platform price. Nothing more."}</h2>
            <p style={{ opacity: ".88", marginTop: "12px", fontSize: "17px" }}>
              {
                "Start with pay as you go, or talk to us about a bundle for your team."
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
