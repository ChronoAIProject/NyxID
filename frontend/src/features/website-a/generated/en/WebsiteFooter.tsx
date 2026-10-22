// Generated from approved A, SHA-256 5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33.
// Run: node scripts/website-a/generate.mjs <baseline.html>
import { SiteLink } from "../../site-link";
import { FooterStatus } from "../../optional-features";
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
            <SiteLink className="brand" href={s.siteHref("/")}>
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
            </SiteLink>
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
            <SiteLink href={s.siteHref("/nyxbot")}>{"NyxBot"}</SiteLink>
            <SiteLink href={s.siteHref("/integrations")}>
              {"Integrations"}
            </SiteLink>
            <SiteLink href={s.siteHref("/how-it-works")}>
              {"How it works"}
            </SiteLink>
            <SiteLink href={s.siteHref("/security")}>{"Security"}</SiteLink>
            <SiteLink href={s.siteHref("/pricing")}>{"Pricing"}</SiteLink>
          </div>
          {"\n      "}
          <div>
            <h4>{"Solutions"}</h4>
            <SiteLink href={s.siteHref("/solutions/restaurant-manager")}>
              {"Restaurant manager"}
            </SiteLink>
            <SiteLink href={s.siteHref("/solutions/help-desk")}>
              {"24-hour help desk"}
            </SiteLink>
            <SiteLink href={s.siteHref("/solutions/back-office-clerk")}>
              {"Back-office clerk"}
            </SiteLink>
            <SiteLink href={s.siteHref("/solutions")}>
              {"All solutions →"}
            </SiteLink>
          </div>
          {"\n      "}
          <div>
            <h4>{"Your agent"}</h4>
            <SiteLink href={s.siteHref("/agents")}>
              {"Connect your agent"}
            </SiteLink>
            <SiteLink href={s.siteHref("/docs/ai/guides/agent-isolation")}>
              {"Scoped keys guide"}
            </SiteLink>
            <SiteLink href={s.siteHref("/docs")}>{"Docs"}</SiteLink>
            <SiteLink href={s.siteHref("/docs/cli/getting-started/install")}>
              {"CLI"}
            </SiteLink>
          </div>
          {"\n      "}
          <div>
            <h4>{"Community"}</h4>
            <SiteLink
              href="https://discord.gg/QMvcs8UQBW"
              target="_blank"
              rel="noopener noreferrer"
            >
              {"Discord"}
            </SiteLink>
            <SiteLink
              href="https://github.com/ChronoAIProject/NyxID"
              target="_blank"
              rel="noopener noreferrer"
            >
              {"GitHub"}
            </SiteLink>
            <SiteLink href={s.siteHref("/blog")}>{"Blog"}</SiteLink>
          </div>
          {"\n      "}
          <div>
            <h4>{"Company"}</h4>
            <SiteLink href={s.siteHref("/contact")}>{"Talk to us"}</SiteLink>
          </div>
          {"\n    "}
        </div>
        {"\n    "}
        <div className="legal small">
          <span>{"© 2026 ChronoAI Pte. Ltd."}</span>
          <span className="legal-links">
            <SiteLink href={s.siteHref("/terms")}>{"Terms"}</SiteLink>
            <SiteLink href={s.siteHref("/privacy")}>{"Privacy"}</SiteLink>
            <FooterStatus />
          </span>
        </div>
        {"\n  "}
      </div>
    </>
  );
}
