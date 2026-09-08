import { useEffect, useRef, useState } from "react";
import { ChevronRight } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { WebDeviceLogin } from "@/components/auth/web-device-login";
import { usePublicConfig } from "@/hooks/use-public-config";
import { useApplyTheme } from "@/hooks/use-theme";
import { openExternal } from "@/lib/navigation";
import { OnboardingNotice } from "./onboarding-shell";
import { BrandIcon } from "./brand-icon";

const SIGN_IN_PROVIDERS = [
  { id: "google", name: "Google" },
  { id: "github", name: "GitHub" },
  { id: "apple", name: "Apple" },
] as const;

export function SignInStep({
  returnTo,
  onContinue,
}: {
  readonly returnTo: string;
  readonly onContinue?: () => void;
}) {
  const { t, i18n } = useTranslation();
  useApplyTheme();
  const config = usePublicConfig();
  const [deviceOpen, setDeviceOpen] = useState(false);
  const [unavailableProvider, setUnavailableProvider] = useState<string | null>(
    null,
  );
  const heading = useRef<HTMLHeadingElement>(null);
  useEffect(() => {
    heading.current?.focus({ preventScroll: true });
    window.scrollTo(0, 0);
  }, []);
  const authLink = (path: string) =>
    `${path}?${new URLSearchParams({ return_to: returnTo })}`;
  return (
    <div className="nyxbot-onboarding nb-auth" lang={i18n.resolvedLanguage}>
      <header className="nb-auth-header">
        <div className="nb-auth-brand">
          <span className="nb-auth-logo">
            <img src="/nyxid-icon-white.svg" alt="" />
          </span>
          <strong>NyxID</strong>
        </div>
      </header>
      <main className="nb-auth-content">
        <h1 ref={heading} tabIndex={-1}>
          {t("signinTitle")}
        </h1>
        <p className="nb-auth-subtitle">{t("signinSubtitle")}</p>
        {onContinue ? (
          <>
            <OnboardingNotice>{t("signedIn")}</OnboardingNotice>
            <Button className="nb-primary nb-full" onClick={onContinue}>
              {t("continue")}
            </Button>
          </>
        ) : (
          <>
            {config.isPending && (
              <OnboardingNotice>{t("loading")}</OnboardingNotice>
            )}
            {config.isError && (
              <OnboardingNotice error>
                {t("signinUnavailable")}{" "}
                <Button onClick={() => void config.refetch()}>
                  {t("retry")}
                </Button>
              </OnboardingNotice>
            )}
            <div className="nb-provider-list">
              {!deviceOpen &&
                SIGN_IN_PROVIDERS.map((p) => {
                  const available =
                    config.isSuccess &&
                    config.data.social_providers.includes(p.id);
                  return (
                    <Button
                      type="button"
                      className="nb-provider"
                      key={p.id}
                      aria-disabled={!available}
                      title={
                        available
                          ? undefined
                          : t("providerUnavailable", { provider: p.name })
                      }
                      onClick={() => {
                        if (!available) {
                          setUnavailableProvider(p.name);
                          return;
                        }
                        setUnavailableProvider(null);
                        openExternal(authLink(`/api/v1/auth/social/${p.id}`));
                      }}
                    >
                      <BrandIcon brand={p.id} />
                      <span>{t("provider", { provider: p.name })}</span>
                      <ChevronRight size={16} />
                    </Button>
                  );
                })}
              <div className="nb-device-login">
                <WebDeviceLogin
                  returnTo={returnTo}
                  isOpen={deviceOpen}
                  onOpenChange={(open) => {
                    setUnavailableProvider(null);
                    setDeviceOpen(open);
                  }}
                  triggerLabel={t("appProvider")}
                  triggerIcon={
                    <span className="nb-auth-logo">
                      <img src="/nyxid-icon-white.svg" alt="" />
                    </span>
                  }
                />
              </div>
            </div>
            {!deviceOpen && unavailableProvider && (
              <OnboardingNotice>
                {t("providerUnavailable", { provider: unavailableProvider })}
              </OnboardingNotice>
            )}
            {!deviceOpen && (
              <p className="nb-signup">
                {t("signupIntro")}{" "}
                <a href={authLink("/register")}>{t("signup")}</a>
              </p>
            )}
            {!deviceOpen && config.data?.email_auth_enabled && (
              <p className="nb-signup">
                <a href={authLink("/login")}>{t("emailSignin")}</a>
              </p>
            )}
          </>
        )}
      </main>
    </div>
  );
}
