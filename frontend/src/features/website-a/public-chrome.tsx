import { ArrowUpRight, ChevronDown, Moon, Sun } from "lucide-react";
import footerDirectory from "../public-site/public-footer-links.json";
import { usePublicTheme } from "../public-site/public-theme";
import { useBrowserContext } from "../public-site/browser-context";
import { authHref as authPath, siteHref as sitePath } from "./auth-links";
import { SiteLink } from "./site-link";
import { BLOG_MENU_ENABLED, STATUS_ENABLED, websiteRelease } from "./release-config";
import "./public-design.css";

type Language = "en" | "zh";
type ChromeProps = {
  language?: Language;
  siteHref?: (path: string) => string;
  authHref?: (kind: "login" | "register") => string;
  localeHref?: (language: Language) => string;
};

export function PublicBrand() {
  return (
    <span className="public-brand">
      <img className="public-brand-mark" src="/nyxid-coloured-icon.svg" alt="" width="24" height="24" />
      <img className="public-brand-day" src="/nyxid-coloured-logo-dark.svg" alt="NyxID" width="88" height="25" />
      <img className="public-brand-night" src="/nyxid-coloured-logo.svg" alt="NyxID" width="88" height="25" />
    </span>
  );
}

export function PublicThemeToggle({ language = "en" }: { language?: Language }) {
  const { theme, toggleTheme } = usePublicTheme();
  return (
    <button className="public-theme-toggle" type="button" onClick={toggleTheme}
      aria-label={language === "zh" ? (theme === "dark" ? "切换日间模式" : "切换夜间模式") : (theme === "dark" ? "Switch to day mode" : "Switch to night mode")}>
      <Moon className="public-moon" size={18} aria-hidden="true" />
      <Sun className="public-sun" size={18} aria-hidden="true" />
    </button>
  );
}

const primaryPages = [
  ["/nyxbot", "NyxBot", "NyxBot", "The app, channels and specialists.", "应用、聊天渠道与专属助手。"],
  ["/solutions", "Solutions", "解决方案", "Bookings, research, calls and office work.", "预订、研究、通话与后台工作。"],
  ["/integrations", "Tools", "工具", "Browse the connected services.", "查看可连接的服务。"],
  ["/pricing", "Pricing", "价格", "Provider costs, credits and your own accounts.", "服务费用、积分与自有账号。"],
] as const;
const morePages = [
  ["/how-it-works", "How it works", "工作原理", "Follow a request through NyxID.", "跟随一次请求了解 NyxID。"],
  ["/agents", "Your agent", "你的智能体", "Connect Claude, Cursor, Codex or your code.", "连接 Claude、Cursor、Codex 或你的代码。"],
  ["/security", "Security", "安全", "Credentials, permissions, approvals and audit.", "凭据、权限、审批与审计。"],
  ["/docs", "Docs", "文档", "Setup guides and API reference.", "设置指南与 API 参考。"],
] as const;

export function PublicHeader({ home = false, route = "", language = "en", siteHref, authHref, localeHref }: ChromeProps & { home?: boolean; route?: string }) {
  const context = useBrowserContext();
  const site = siteHref ?? ((path: string) => sitePath(path, context.search));
  const auth = authHref ?? ((kind: "login" | "register") => authPath(kind, context.search));
  const zh = language === "zh";
  const anchors: Record<string, string> = { "/nyxbot": "#pile", "/solutions": "#jobs", "/integrations": "#apps" };
  const links = primaryPages.map(([path, en, cn]) => [home && anchors[path] ? anchors[path]! : site(path), zh ? cn : en, path] as const);
  return (
    <header id="nav" className={`public-header${home ? " nav" : ""}`} data-nav={home ? "" : undefined}>
      <div className="public-wrap public-nav-bar">
        <div className="public-nav-box">
          <SiteLink className="public-home-link" href={home ? "#top" : site("/")} aria-label={zh ? "NyxID 首页" : "NyxID home"}><PublicBrand /></SiteLink>
        </div>
        <div className="public-nav-box public-nav-right">
          <nav className="public-nav-links" aria-label={zh ? "主导航" : "Main"}>
            {links.map(([href, label, path]) => <SiteLink key={path} href={href} aria-current={route === path ? "page" : route.startsWith(`${path}/`) ? "location" : undefined}>{label}</SiteLink>)}
          </nav>
          <details key={`${route}:${language}`} className="public-more"
            onBlur={(e) => { if (!e.currentTarget.contains(e.relatedTarget)) e.currentTarget.open = false; }}
            onKeyDown={(e) => { if (e.key === "Escape") { e.currentTarget.open = false; e.currentTarget.querySelector("summary")?.focus(); } }}>
            <summary className="list-none">{zh ? "了解更多" : "Show more"}<ChevronDown size={14} aria-hidden="true" /></summary>
            <nav className="public-more-menu" aria-label={zh ? "探索 NyxID" : "Explore NyxID"} onClick={(e) => {
              if (e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
              if ((e.target as Element).closest("a")) e.currentTarget.closest("details")?.removeAttribute("open");
            }}>
              {primaryPages.map(([path, title, titleZh]) => <SiteLink className="public-more-primary" key={path} href={site(path)} aria-current={route === path ? "page" : route.startsWith(`${path}/`) ? "location" : undefined}><b>{zh ? titleZh : title}</b><ArrowUpRight size={17} aria-hidden="true" /></SiteLink>)}
              {home && <SiteLink className="public-more-home-nyxbot" href={site("/nyxbot")}><span><b>NyxBot</b><small>{zh ? "查看应用与专属助手。" : "See the app and specialists."}</small></span><ArrowUpRight size={17} aria-hidden="true" /></SiteLink>}
              {morePages.map(([path, title, titleZh, detail, detailZh]) => (
                <SiteLink key={path} href={site(path)} aria-current={route === path ? "page" : undefined}>
                  <span><b>{zh ? titleZh : title}</b><small>{zh ? detailZh : detail}</small></span><ArrowUpRight size={17} aria-hidden="true" />
                </SiteLink>
              ))}
              {BLOG_MENU_ENABLED && <SiteLink data-nav="blog" href={site("/blog")}>{zh ? "博客" : "Blog"}<ArrowUpRight size={17} aria-hidden="true" /></SiteLink>}
              <div className="public-more-bottom">
                <a href={auth("login")}>{zh ? "登录" : "Log in"}</a>
                <SiteLink href={site("/contact")}>{zh ? "联系" : "Contact"}</SiteLink>
                {localeHref && <SiteLink href={localeHref(zh ? "en" : "zh")} lang={zh ? "en" : "zh-CN"}>{zh ? "EN" : "中文"}</SiteLink>}
              </div>
            </nav>
          </details>
          <span className="public-nav-divider" aria-hidden="true" />
          <a className="public-login" href={auth("login")}>{zh ? "登录" : "Log in"}</a>
          <PublicThemeToggle language={language} />
          <a className="public-cta" href={auth("register")}>{zh ? "开始使用" : "Get started"}</a>
        </div>
      </div>
    </header>
  );
}

const footerGroups = footerDirectory as { title: string; zh: string; links: [string, string, string][] }[];

export function PublicFooter({ language = "en", siteHref, localeHref, showStatus = true }: Pick<ChromeProps, "language" | "siteHref" | "localeHref"> & { showStatus?: boolean }) {
  const context = useBrowserContext();
  const site = siteHref ?? ((path: string) => sitePath(path, context.search));
  const zh = language === "zh";
  return (
    <footer id="foot" className="public-footer" aria-label={zh ? "网站页脚" : "Site footer"}>
      <div className="public-wrap">
        <div className="public-footer-grid">
          <div className="public-footer-brand">
            <SiteLink href={site("/")} aria-label={zh ? "NyxID 首页" : "NyxID home"}><PublicBrand /></SiteLink>
            <p>{zh ? "连接智能体与工具的开源网关。" : "An open-source gateway for agents and their tools."}</p>
          </div>
          {footerGroups.map((group) => <nav key={group.title} aria-label={zh ? group.zh : group.title}>
            <h2>{zh ? group.zh : group.title}</h2>
            {group.links.map(([path, title, titleZh]) => <SiteLink key={path} href={path.startsWith("/") ? site(path) : path} {...(path.startsWith("https:") ? { target: "_blank", rel: "noopener noreferrer" } : {})}>{zh ? titleZh : title}</SiteLink>)}
          </nav>)}
        </div>
        <div className="public-footer-legal">
          <span>© 2026 ChronoAI Pte. Ltd.</span>
          <nav aria-label={zh ? "法律" : "Legal"}>
            <a href={site("/terms")}>{zh ? "服务条款" : "Terms"}</a>
            <a href={site("/privacy")}>{zh ? "隐私政策" : "Privacy"}</a>
            {STATUS_ENABLED && showStatus && websiteRelease.footerStatus && <a data-website-feature="footer-status" href={websiteRelease.footerStatus} target="_blank" rel="noopener noreferrer">{zh ? "服务状态" : "Status"}</a>}
            {localeHref && <SiteLink href={localeHref(zh ? "en" : "zh")} lang={zh ? "en" : "zh-CN"}>{zh ? "English" : "中文"}</SiteLink>}
          </nav>
        </div>
      </div>
    </footer>
  );
}
