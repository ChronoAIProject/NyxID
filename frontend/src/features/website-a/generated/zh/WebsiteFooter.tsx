// Generated from approved A, SHA-256 5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33.
// Run: node scripts/website-a/generate.mjs <baseline.html>
import { SiteLink } from "../../site-link";
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
                "开源的智能体网关。安全地把 AI 连接到你的服务和 API，让 NyxBot 替你干活。"
              }
            </p>
          </div>
          {"\n      "}
          <div>
            <h4>{"产品"}</h4>
            <SiteLink href={s.siteHref("/nyxbot")}>{"NyxBot"}</SiteLink>
            <SiteLink href={s.siteHref("/integrations")}>{"集成"}</SiteLink>
            <SiteLink href={s.siteHref("/how-it-works")}>{"工作原理"}</SiteLink>
            <SiteLink href={s.siteHref("/security")}>{"安全"}</SiteLink>
            <SiteLink href={s.siteHref("/pricing")}>{"价格"}</SiteLink>
          </div>
          {"\n      "}
          <div>
            <h4>{"方案"}</h4>
            <SiteLink href={s.siteHref("/solutions/restaurant-manager")}>
              {"餐厅经理"}
            </SiteLink>
            <SiteLink href={s.siteHref("/solutions/help-desk")}>
              {"24 小时客服"}
            </SiteLink>
            <SiteLink href={s.siteHref("/solutions/back-office-clerk")}>
              {"后台文员"}
            </SiteLink>
            <SiteLink href={s.siteHref("/solutions")}>{"全部方案 →"}</SiteLink>
          </div>
          {"\n      "}
          <div>
            <h4>{"接入智能体"}</h4>
            <SiteLink href={s.siteHref("/agents")}>{"接入你的智能体"}</SiteLink>
            <SiteLink href={s.siteHref("/docs/ai/guides/agent-isolation")}>
              {"限权密钥指南"}
            </SiteLink>
            <SiteLink href={s.siteHref("/docs")}>{"Docs"}</SiteLink>
            <SiteLink href={s.siteHref("/docs/cli/getting-started/install")}>
              {"CLI"}
            </SiteLink>
          </div>
          {"\n      "}
          <div>
            <h4>{"社区"}</h4>
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
            <SiteLink href={s.siteHref("/blog")}>{"博客"}</SiteLink>
          </div>
          {"\n      "}
          <div>
            <h4>{"公司"}</h4>
            <SiteLink href={s.siteHref("/contact")}>{"联系我们"}</SiteLink>
          </div>
          {"\n    "}
        </div>
        {"\n    "}
        <div className="legal small">
          <span>{"© 2026 ChronoAI Pte. Ltd."}</span>
          <span className="legal-links">
            <SiteLink href={s.siteHref("/terms")}>{"条款"}</SiteLink>
            <SiteLink href={s.siteHref("/privacy")}>{"隐私"}</SiteLink>
          </span>
        </div>
        {"\n  "}
      </div>
    </>
  );
}
