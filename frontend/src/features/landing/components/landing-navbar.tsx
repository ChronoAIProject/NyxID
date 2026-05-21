import { useState } from "react";
import { useTranslation } from "react-i18next";
import { GitHubButton } from "@/components/shared/github-button";
import { LanguageSwitcher } from "./language-switcher";
import { useScroll } from "../hooks/use-scroll";
import { useAuthStore } from "@/stores/auth-store";

export function LandingNavbar() {
  const [scrolled, setScrolled] = useState(false);
  const { t } = useTranslation();
  const isAuthenticated = useAuthStore((s) => s.isAuthenticated);

  useScroll((scrollY) => setScrolled(scrollY > 0));

  const search =
    typeof window !== "undefined" ? window.location.search : "";
  const loginHref = `/login${search}`;
  const registerHref = `/register${search}`;

  return (
    <nav
      className={`fixed top-0 left-0 right-0 z-50 border-b backdrop-blur-md transition-colors duration-300 ${
        scrolled
          ? "border-border/60 bg-landing-bg/80"
          : "border-transparent bg-landing-bg/60"
      }`}
    >
      <div className="mx-auto flex max-w-6xl items-center justify-between px-6 py-4">
        <a href={isAuthenticated ? "/dashboard" : "#"} className="flex items-center">
          <img src="/nyxid-wordmark.svg" alt="NyxID" className="h-8 w-auto" />
        </a>

        <div className="flex items-center gap-3">
          <a
            href="/docs"
            className="hidden text-sm font-medium text-gray-300 transition-colors hover:text-white sm:inline-block"
          >
            {t("nav.docs")}
          </a>
          <GitHubButton className="text-text-tertiary hover:text-foreground" />
          <LanguageSwitcher />
          <a
            href={loginHref}
            className="rounded-lg border border-white/[0.08] px-5 py-2 text-sm font-medium text-foreground transition-colors hover:border-white/[0.15] hover:bg-white/[0.03]"
          >
            {t("nav.login")}
          </a>
          <a
            href={registerHref}
            className="hidden rounded-lg nyx-gradient-vivid px-5 py-2 text-sm font-medium text-white shadow-[0_0_12px_rgba(90,42,241,0.25)] transition-all hover:shadow-[0_0_18px_rgba(90,42,241,0.35)] hover:brightness-110 md:inline-block"
          >
            {t("nav.register")}
          </a>
        </div>
      </div>
    </nav>
  );
}
