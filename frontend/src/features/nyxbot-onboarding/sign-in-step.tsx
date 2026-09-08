import { useState } from "react";
import { ChevronRight } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { WebDeviceLogin } from "@/components/auth/web-device-login";
import { usePublicConfig } from "@/hooks/use-public-config";
import { openExternal } from "@/lib/navigation";
import {
  OnboardingShell,
  OnboardingNotice,
  StepHelp,
  BackButton,
} from "./onboarding-shell";
import { BrandIcon } from "./brand-icon";

export function SignInStep({
  returnTo,
  onContinue,
}: {
  readonly returnTo: string;
  readonly onContinue?: () => void;
}) {
  const { t } = useTranslation();
  const config = usePublicConfig();
  const [deviceOpen, setDeviceOpen] = useState(false);
  const authLink = (path: string) =>
    `${path}?${new URLSearchParams({ return_to: returnTo })}`;
  const providers = [
    { id: "google", name: "Google" },
    { id: "github", name: "GitHub" },
    { id: "apple", name: "Apple" },
  ].filter((p) => config.data?.social_providers.includes(p.id));
  return (
    <OnboardingShell
      title={t("signinTitle")}
      subtitle={t("signinSubtitle")}
      step={1}
      actions={
        <>
          {onContinue ? (
            <Button className="nb-primary" onClick={onContinue}>
              {t("continue")}
            </Button>
          ) : (
            <Button className="nb-primary" asChild>
              <a href={authLink("/login")}>{t("continue")}</a>
            </Button>
          )}
          <BackButton
            onClick={() => window.history.back()}
            disabled={Boolean(onContinue) || window.history.length <= 1}
          />
        </>
      }
    >
      <p className="nb-intro">
        {t("signinIntro")} <StepHelp>{t("signinHelp")}</StepHelp>
      </p>
      {onContinue ? (
        <OnboardingNotice>{t("signedIn")}</OnboardingNotice>
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
              providers.map((p) => (
                <Button
                  className="nb-provider"
                  key={p.id}
                  onClick={() =>
                    openExternal(authLink(`/api/v1/auth/social/${p.id}`))
                  }
                >
                  <BrandIcon brand={p.id} />
                  <span>{t("provider", { provider: p.name })}</span>
                  <ChevronRight size={16} />
                </Button>
              ))}
            <div className="nb-device-login">
              <WebDeviceLogin
                returnTo={returnTo}
                isOpen={deviceOpen}
                onOpenChange={setDeviceOpen}
                triggerLabel={t("appProvider")}
              />
            </div>
          </div>
          {!deviceOpen && (
            <p className="nb-signup">
              {t("signupIntro")}{" "}
              <a href={authLink("/register")}>{t("signup")}</a>
            </p>
          )}
          {config.data?.email_auth_enabled && (
            <p className="nb-signup">
              <a href={authLink("/login")}>{t("emailSignin")}</a>
            </p>
          )}
        </>
      )}
    </OnboardingShell>
  );
}
