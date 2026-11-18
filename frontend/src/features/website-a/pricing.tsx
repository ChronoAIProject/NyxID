import copy from "./pricing-copy.json";
import { SiteLink } from "./site-link";
import { VisualLink } from "./visuals/elements";
import { useWebsite } from "./website-context";

// Effective pricing policy: chrono-finance#315, decisions D2, D3 and D5.
export function PricingSection() {
  const s = useWebsite();
  const c = copy[s.language];

  return (
    <>
      <section className="hero glowbg" style={{ position: "relative" }}>
        <div className="wrap">
          <div className="over v">{c.label}</div>
          <h1
            className="h1"
            style={{ marginTop: "14px", fontSize: "var(--fs-h1)" }}
          >
            {c.heading}
            <br />
            <span className="grad">{c.highlight}</span>
          </h1>
          <p className="lead">{c.description}</p>
          <div className="cta">
            <VisualLink
              className="btn p lg"
              href={s.authHref("register")}
              effect="button"
            >
              {c.start}
            </VisualLink>
            <SiteLink className="btn lg" href={s.siteHref("/contact")}>
              {c.talk}
            </SiteLink>
          </div>
        </div>
      </section>
      <section className="s">
        <div className="wrap split">
          <div>
            <div className="over v">{c.commitmentLabel}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {c.commitmentHeading}{" "}
              <span className="grad">{c.commitmentHighlight}</span>
            </h2>
            <p className="lead" style={{ marginTop: "14px" }}>
              {c.commitmentDescription}
            </p>
            <ul className="checks" style={{ marginTop: "24px" }}>
              {c.checks.map((check) => (
                <li key={check.title}>
                  <span className="ck">
                    <svg className="i" viewBox="0 0 24 24" aria-hidden="true">
                      <path d="M20 6 9 17l-5-5" />
                    </svg>
                  </span>
                  <div>
                    <b>{check.title}</b>
                    <span>{check.text}</span>
                  </div>
                </li>
              ))}
            </ul>
          </div>
          <div className="card" style={{ padding: "28px" }}>
            <div className="over">{c.exampleLabel}</div>
            <div style={{ display: "grid", marginTop: "18px" }}>
              {c.exampleRows.map((row) => (
                <div
                  key={row.label}
                  style={{
                    display: "flex",
                    justifyContent: "space-between",
                    alignItems: "center",
                    gap: "16px",
                    padding: "12px 0",
                    borderBottom: "1px solid var(--border)",
                    ...(row.total
                      ? { fontWeight: "700", fontSize: "18px" }
                      : {}),
                  }}
                >
                  <span>{row.label}</span>
                  <span className="mono" style={{ whiteSpace: "nowrap" }}>
                    {row.amount}
                  </span>
                </div>
              ))}
            </div>
            <p className="small" style={{ marginTop: "14px" }}>
              {c.exampleNote}
            </p>
          </div>
        </div>
      </section>
      <section className="s">
        <div className="wrap">
          <div
            className="center"
            style={{ textAlign: "center", maxWidth: "720px" }}
          >
            <div className="over v">{c.pricingLabel}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {c.pricingHeading}
            </h2>
          </div>
          <div className="grid g3" style={{ marginTop: "40px" }}>
            {c.cards.map((card, index) => (
              <div className="card" key={card.title}>
                <span className={`badge ${["b-ok", "b-v", "b-m"][index]}`}>
                  {card.badge}
                </span>
                <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                  {card.title}
                </h3>
                <p className="sub" style={{ fontSize: "14.5px" }}>
                  {card.text}
                </p>
              </div>
            ))}
          </div>
          <p
            className="small center"
            style={{ textAlign: "center", marginTop: "20px" }}
          >
            {c.freeNote}
          </p>
        </div>
      </section>
      <section className="s">
        <div className="wrap" style={{ maxWidth: "860px" }}>
          <div className="over v">{c.faqLabel}</div>
          <h2 className="h2" style={{ margin: "10px 0 18px" }}>
            {c.faqHeading}
          </h2>
          {c.faqs.map((faq) => (
            <details key={faq.question}>
              <summary>{faq.question}</summary>
              <p>{faq.answer}</p>
            </details>
          ))}
        </div>
      </section>
      <section className="s">
        <div className="wrap">
          <div className="band">
            <h2 className="h2">{c.ctaHeading}</h2>
            <p style={{ opacity: ".88", marginTop: "12px", fontSize: "17px" }}>
              {c.ctaDescription}
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
                {c.start}
              </SiteLink>
              <SiteLink className="btn lg gh" href={s.siteHref("/solutions")}>
                {c.browse}
              </SiteLink>
            </div>
          </div>
        </div>
      </section>
    </>
  );
}
