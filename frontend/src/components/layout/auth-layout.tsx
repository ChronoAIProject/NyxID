import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { useRef } from "react";
import { Link, Outlet } from "@tanstack/react-router";
import { GitHubButton } from "@/components/shared/github-button";

import { useLogoHref } from "@/hooks/use-logo-href";
import { useApplyTheme } from "@/hooks/use-theme";
import { PublicBrand, PublicThemeToggle } from "@/features/public-site/public-chrome";
import { usePublicThemeElement } from "@/features/public-site/public-theme";
import "@/features/website-a/public-reading.css";

/**
 * Sign-in and sign-up wear the public site's material: the form on the paper and
 * the landing page's closing painting beside it. The app's text size and density
 * still apply; colours follow the public day/night choice, which never changes
 * the app's own theme preference.
 */
export function AuthLayout() {
  const { i18n } = useUiTranslation();
  useApplyTheme();
  const scope = useRef<HTMLDivElement>(null);
  usePublicThemeElement(scope);
  const logoHref = useLogoHref();
  const language = i18n.resolvedLanguage?.startsWith("zh") ? "zh" : "en";

  return (
    <div ref={scope} className="nyx-public public-reading public-auth flex h-dvh overflow-hidden">
      {/* Left — form column */}
      <div
        className="public-auth-form relative flex w-full shrink-0 flex-col lg:w-[calc(40%-20px)]"
        style={{
          paddingTop: "max(0px, var(--sat))",
          paddingBottom: "max(0px, var(--sab))",
          paddingLeft: "max(0px, var(--sal))",
        }}
      >
        {/* Logo and day/night switch — top */}
        <div className="public-auth-top shrink-0 px-4 pt-8 sm:px-10">
          <Link to={logoHref} className="public-auth-home">
            <PublicBrand />
          </Link>
          <PublicThemeToggle language={language} />
        </div>

        {/* Form area — vertically centered */}
        <div className="flex flex-1 items-center justify-center overflow-y-auto px-4 py-10 sm:px-10">
          <div className="w-full max-w-[400px]">
            <Outlet />
          </div>
        </div>

        {/* Footer — sticky bottom */}
        <div className="mt-auto shrink-0 px-4 pb-8 sm:px-10">
          <div className="mb-4 flex justify-center">
            <GitHubButton
              label={translateUi("View source on GitHub")}
              size={16}
              className="type-secondary-emphasis text-muted-foreground hover:text-foreground"
            />
          </div>
          <p className="text-center text-13 text-muted-foreground">{translateUi("By continuing, you agree to NyxID's")}{" "}
            <a href="/terms" className="text-muted-foreground underline underline-offset-2 hover:text-foreground">{translateUi("Terms of Service")}</a>{" "}{translateUi("and")}<br />
            <a href="/privacy" className="text-muted-foreground underline underline-offset-2 hover:text-foreground">{translateUi("Privacy Policy")}</a>{translateUi(".")}</p>
        </div>
      </div>

      {/* Right — the halftone Nyx from the landing page's close, one still per mode (hidden on mobile) */}
      <div className="public-auth-art relative hidden lg:block lg:w-[calc(60%+20px)]" />
    </div>
  );
}
