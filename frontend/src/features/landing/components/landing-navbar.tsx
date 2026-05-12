import { useState } from "react";
import { useTranslation } from "react-i18next";
import { LanguageSwitcher } from "./language-switcher";
import { useScroll } from "../hooks/use-scroll";

export function LandingNavbar() {
  const [scrolled, setScrolled] = useState(false);
  const { t } = useTranslation();

  useScroll((scrollY) => setScrolled(scrollY > 0));

  const search =
    typeof window !== "undefined" ? window.location.search : "";
  const loginHref = `/login${search}`;
  const registerHref = `/register${search}`;

  return (
    <nav
      className={`fixed top-0 left-0 right-0 z-50 border-b backdrop-blur-md transition-all duration-500 ${
        scrolled
          ? "border-primary/20 bg-landing-bg/80 navbar-glow"
          : "border-landing-border-subtle bg-landing-bg/80"
      }`}
    >
      <div className="mx-auto flex max-w-6xl items-center justify-between px-6 py-4">
        <a href="#" className="flex items-center">
          <img src="/nyxid-wordmark.svg" alt="NyxID" className="h-8 w-auto" />
        </a>

        <div className="flex items-center gap-3">
          <LanguageSwitcher />
          <a
            href={loginHref}
            className="rounded-lg border border-primary/40 px-5 py-2 text-sm font-semibold text-white transition-colors hover:bg-primary/10"
          >
            {t("nav.login")}
          </a>
          <a
            href={registerHref}
            className="hidden rounded-lg bg-primary px-5 py-2 text-sm font-semibold text-white transition-colors hover:bg-void-400 md:inline-block"
          >
            {t("nav.register")}
          </a>
        </div>
      </div>
    </nav>
  );
}
