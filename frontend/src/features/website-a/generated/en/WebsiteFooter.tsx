// Generated from approved A, SHA-256 5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33.
// Run: node scripts/website-a/generate.mjs <baseline.html>
import { useWebsite } from "../../website-context";
export default function WebsiteFooter() {
  const s = useWebsite();
  void s;
  return (
    <>
      <div className="wrap">
        {"\n    "}
        <div className="fgrid">
          {"\n      "}
          <div>
            <a className="brand" href={s.siteHref("/")}>
              <img
                className="ld"
                src="/website-a/b9c156ac563aec44.svg"
                alt="NyxID"
                onError={s.imageFallback}
              />
              <img
                className="ll"
                src="/website-a/d69537849bcfe952.svg"
                alt="NyxID"
                onError={s.imageFallback}
              />
            </a>
            <p
              className="small"
              style={{ marginTop: "14px", maxWidth: "270px" }}
            >
              {
                "The open-source agent gateway. Connect AI to your services and APIs, safely, and let NyxBot do the work."
              }
            </p>
          </div>
          {"\n      "}
          <div>
            <h4>{"Product"}</h4>
            <a href={s.siteHref("/nyxbot")}>{"NyxBot"}</a>
            <a href={s.siteHref("/integrations")}>{"Integrations"}</a>
            <a href={s.siteHref("/how-it-works")}>{"How it works"}</a>
            <a href={s.siteHref("/security")}>{"Security"}</a>
            <a href={s.siteHref("/pricing")}>{"Pricing"}</a>
          </div>
          {"\n      "}
          <div>
            <h4>{"Solutions"}</h4>
            <a href={s.siteHref("/solutions/restaurant-manager")}>
              {"Restaurant manager"}
            </a>
            <a href={s.siteHref("/solutions/help-desk")}>
              {"24-hour help desk"}
            </a>
            <a href={s.siteHref("/solutions/back-office-clerk")}>
              {"Back-office clerk"}
            </a>
            <a href={s.siteHref("/solutions")}>{"All solutions →"}</a>
          </div>
          {"\n      "}
          <div>
            <h4>{"Your agent"}</h4>
            <a href={s.siteHref("/agents")}>{"Connect your agent"}</a>
            <a href={s.siteHref("/blog/scoped-keys")}>{"Scoped keys guide"}</a>
            <a href={s.siteHref("/docs")}>{"Docs"}</a>
            <a href={s.siteHref("/docs/cli/getting-started/install")}>
              {"CLI"}
            </a>
          </div>
          {"\n      "}
          <div>
            <h4>{"Community"}</h4>
            <a
              href="https://discord.gg/QMvcs8UQBW"
              target="_blank"
              rel="noopener noreferrer"
            >
              {"Discord"}
            </a>
            <a
              href="https://github.com/ChronoAIProject/NyxID"
              target="_blank"
              rel="noopener noreferrer"
            >
              {"GitHub"}
            </a>
            <a href={s.siteHref("/blog")}>{"Blog"}</a>
          </div>
          {"\n      "}
          <div>
            <h4>{"Company"}</h4>
            <a href={s.siteHref("/contact")}>{"Talk to us"}</a>
          </div>
          {"\n    "}
        </div>
        {"\n    "}
        <div className="legal small">
          <span>{"© 2026 ChronoAI Pte. Ltd."}</span>
          <span className="legal-links">
            <a href={s.siteHref("/terms")}>{"Terms"}</a>
            <a href={s.siteHref("/privacy")}>{"Privacy"}</a>
          </span>
        </div>
        {"\n  "}
      </div>
    </>
  );
}
