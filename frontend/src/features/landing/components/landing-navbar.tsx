import { useState } from "react";
import { useTranslation } from "react-i18next";
import { PortalMarkLogo } from "@/components/shared/portal-mark-logo";
import { LanguageSwitcher } from "./language-switcher";
import { useScroll } from "../hooks/use-scroll";

export function LandingNavbar() {
  const [scrolled, setScrolled] = useState(false);
  const { t } = useTranslation();

  useScroll((scrollY) => setScrolled(scrollY > 0));

  return (
    <nav
      className={`fixed top-0 left-0 right-0 z-50 border-b backdrop-blur-md transition-all duration-500 ${
        scrolled
          ? "border-primary/20 bg-landing-bg/80 navbar-glow"
          : "border-landing-border-subtle bg-landing-bg/80"
      }`}
    >
      <div className="mx-auto flex max-w-6xl items-center justify-between px-6 py-4">
        <a href="#" className="flex items-center gap-2">
          <PortalMarkLogo size={32} />
          <span className="font-serif text-xl text-white">NyxID</span>
        </a>

        <div className="flex items-center gap-3">
          <LanguageSwitcher />
          <a
            href="#waitlist"
            className="rounded-lg bg-primary px-5 py-2 text-sm font-semibold text-white transition-colors hover:bg-void-400"
          >
            {t("nav.joinWaitlist")}
          </a>
        </div>
      </div>
    </nav>
  );
}
