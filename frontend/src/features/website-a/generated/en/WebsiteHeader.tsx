// Generated from approved A, SHA-256 5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33.
// Run: node scripts/website-a/generate.mjs <baseline.html>
import { VisualLink } from "../../visuals/elements";
import { useWebsite } from "../../website-context";
export default function WebsiteHeader() {
  const s = useWebsite();
  void s;
  return (
    <>
      <div className="wrap">
        {"\n    "}
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
        {"\n    "}
        <div
          className="links"
          style={{ display: "contents" }}
          id="website-nav-links"
        >
          {"\n      "}
          <a
            className="nl"
            href={s.siteHref("/nyxbot")}
            aria-current={s.route === "/nyxbot" ? "page" : undefined}
            data-nav="nyxbot"
          >
            {"NyxBot"}
          </a>
          {"\n      "}
          <a
            className="nl"
            href={s.siteHref("/solutions")}
            aria-current={s.route === "/solutions" ? "page" : undefined}
            data-nav="solutions"
          >
            {"Solutions"}
          </a>
          {"\n      "}
          <a
            className="nl"
            href={s.siteHref("/how-it-works")}
            aria-current={s.route === "/how-it-works" ? "page" : undefined}
            data-nav="how-it-works"
          >
            {"How it works"}
          </a>
          {"\n      "}
          <a
            className="nl"
            href={s.siteHref("/security")}
            aria-current={s.route === "/security" ? "page" : undefined}
            data-nav="security"
          >
            {"Security"}
          </a>
          {"\n      "}
          <a
            className="nl"
            href={s.siteHref("/agents")}
            aria-current={s.route === "/agents" ? "page" : undefined}
            data-nav="agents"
          >
            {"Your agent"}
          </a>
          {"\n      "}
          <a
            className="nl"
            href={s.siteHref("/pricing")}
            aria-current={s.route === "/pricing" ? "page" : undefined}
            data-nav="pricing"
          >
            {"Pricing"}
          </a>
          {"\n      "}
          <a
            className="nl"
            href={s.siteHref("/blog")}
            aria-current={s.route === "/blog" ? "page" : undefined}
            data-nav="blog"
          >
            {"Blog"}
          </a>
          {"\n      "}
          <a
            className="nl m-only"
            href="https://discord.gg/QMvcs8UQBW"
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
              <path d="M20.317 4.37a19.791 19.791 0 0 0-4.885-1.515.074.074 0 0 0-.079.037c-.21.375-.444.864-.608 1.25a18.27 18.27 0 0 0-5.487 0 12.64 12.64 0 0 0-.617-1.25.077.077 0 0 0-.079-.037A19.736 19.736 0 0 0 3.677 4.37a.07.07 0 0 0-.032.027C.533 9.046-.32 13.58.099 18.057a.082.082 0 0 0 .031.057 19.9 19.9 0 0 0 5.993 3.03.078.078 0 0 0 .084-.028c.462-.63.874-1.295 1.226-1.994a.076.076 0 0 0-.041-.106 13.107 13.107 0 0 1-1.872-.892.077.077 0 0 1-.008-.128 10.2 10.2 0 0 0 .372-.292.074.074 0 0 1 .077-.01c3.928 1.793 8.18 1.793 12.062 0a.074.074 0 0 1 .078.01c.12.098.246.198.373.292a.077.077 0 0 1-.006.127 12.299 12.299 0 0 1-1.873.892.077.077 0 0 0-.041.107c.36.698.772 1.362 1.225 1.993a.076.076 0 0 0 .084.028 19.839 19.839 0 0 0 6.002-3.03.077.077 0 0 0 .032-.054c.5-5.177-.838-9.674-3.549-13.66a.061.061 0 0 0-.031-.03zM8.02 15.33c-1.183 0-2.157-1.085-2.157-2.419 0-1.333.956-2.419 2.157-2.419 1.21 0 2.176 1.095 2.157 2.42 0 1.333-.956 2.418-2.157 2.418zm7.975 0c-1.183 0-2.157-1.085-2.157-2.419 0-1.333.956-2.419 2.157-2.419 1.21 0 2.176 1.095 2.157 2.42 0 1.333-.947 2.418-2.157 2.418z"></path>
            </svg>
            {"  Discord"}
          </a>
          {"\n    "}
          <a className="nl m-only" href={s.authHref("login")}>
            {"Log in"}
          </a>
        </div>
        {"\n    "}
        <span className="sp"></span>
        {"\n    "}
        <a
          className="iconbtn"
          href="https://discord.gg/QMvcs8UQBW"
          target="_blank"
          rel="noopener noreferrer"
          aria-label="Discord"
          title="Join our Discord"
        >
          <svg
            className="dc"
            width="18"
            height="18"
            viewBox="0 0 24 24"
            aria-hidden="true"
          >
            <path d="M20.317 4.37a19.791 19.791 0 0 0-4.885-1.515.074.074 0 0 0-.079.037c-.21.375-.444.864-.608 1.25a18.27 18.27 0 0 0-5.487 0 12.64 12.64 0 0 0-.617-1.25.077.077 0 0 0-.079-.037A19.736 19.736 0 0 0 3.677 4.37a.07.07 0 0 0-.032.027C.533 9.046-.32 13.58.099 18.057a.082.082 0 0 0 .031.057 19.9 19.9 0 0 0 5.993 3.03.078.078 0 0 0 .084-.028c.462-.63.874-1.295 1.226-1.994a.076.076 0 0 0-.041-.106 13.107 13.107 0 0 1-1.872-.892.077.077 0 0 1-.008-.128 10.2 10.2 0 0 0 .372-.292.074.074 0 0 1 .077-.01c3.928 1.793 8.18 1.793 12.062 0a.074.074 0 0 1 .078.01c.12.098.246.198.373.292a.077.077 0 0 1-.006.127 12.299 12.299 0 0 1-1.873.892.077.077 0 0 0-.041.107c.36.698.772 1.362 1.225 1.993a.076.076 0 0 0 .084.028 19.839 19.839 0 0 0 6.002-3.03.077.077 0 0 0 .032-.054c.5-5.177-.838-9.674-3.549-13.66a.061.061 0 0 0-.031-.03zM8.02 15.33c-1.183 0-2.157-1.085-2.157-2.419 0-1.333.956-2.419 2.157-2.419 1.21 0 2.176 1.095 2.157 2.42 0 1.333-.956 2.418-2.157 2.418zm7.975 0c-1.183 0-2.157-1.085-2.157-2.419 0-1.333.956-2.419 2.157-2.419 1.21 0 2.176 1.095 2.157 2.42 0 1.333-.947 2.418-2.157 2.418z"></path>
          </svg>
        </a>
        {"\n    "}
        <div className="lang" role="group" aria-label="Language">
          <button
            data-lang="en"
            className={s.language === "en" ? "on" : ""}
            type="button"
            onClick={() => s.setLanguage("en")}
          >
            {"EN"}
          </button>
          <button
            data-lang="zh"
            className={s.language === "zh" ? "on" : ""}
            type="button"
            onClick={() => s.setLanguage("zh")}
          >
            {"中文"}
          </button>
        </div>
        {"\n    "}
        <a className="nl hide-m" href={s.authHref("login")}>
          {"Log in"}
        </a>
        {"\n    "}
        <VisualLink
          className="btn p sm"
          href={s.authHref("register")}
          effect="button"
        >
          {"Get started"}
        </VisualLink>
        {"\n    "}
        <button
          className="btn sm burger"
          id="burger"
          type="button"
          onClick={s.toggleMenu}
          aria-expanded={s.menuOpen}
          aria-controls="website-nav-links"
        >
          {"Menu"}
        </button>
        {"\n  "}
      </div>
    </>
  );
}
