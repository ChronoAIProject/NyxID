import { useState } from "react";
import { I18nextProvider, useTranslation } from "react-i18next";
import { Check } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useAuthStore } from "@/stores/auth-store";
import { useNyxbotOnboarding } from "@/hooks/use-nyxbot-onboarding";
import {
  nyxbotSearchSchema,
  type NyxbotChannel,
} from "@/schemas/nyxbot-onboarding";
import { nyxbotI18n } from "@/features/nyxbot-onboarding/i18n";
import { OnboardingNotice } from "@/features/nyxbot-onboarding/onboarding-shell";
import { DataSourceStep } from "@/features/nyxbot-onboarding/data-source-step";
import { SignInStep } from "@/features/nyxbot-onboarding/sign-in-step";
import { ChannelStep } from "@/features/nyxbot-onboarding/channel-step";
import { SpendingCapStep } from "@/features/nyxbot-onboarding/spending-cap-step";
import { LinkChannelStep } from "@/features/nyxbot-onboarding/link-channel-step";
import "@/features/nyxbot-onboarding/onboarding.css";

function ConnectedOnboarding({
  userId,
  accountName,
  referral,
  callbackFailed,
}: {
  readonly userId: string;
  readonly accountName: string;
  readonly referral?: NyxbotChannel;
  readonly callbackFailed: boolean;
}) {
  const { t } = useTranslation();
  const flow = useNyxbotOnboarding(userId, referral);
  const [view, setView] = useState<
    "account" | "source" | "channel" | "cap" | "link"
  >(() => (flow.progress.botId ? "link" : "source"));
  const loading =
    flow.keys.isPending ||
    flow.catalog.isPending ||
    (Boolean(flow.progress.googleKeyId) && flow.authorization.isPending);
  const loadError =
    flow.keys.isError || flow.catalog.isError || flow.authorization.isError;
  // A URL or locally remembered step never substitutes for a live authorization.
  const sourceReady = Boolean(flow.connectedKey) && !loadError;

  if (view === "account")
    return (
      <SignInStep
        returnTo={window.location.href}
        onContinue={() => setView("source")}
      />
    );

  if (view === "link" && flow.progress.botId)
    return (
      <LinkChannelStep
        botId={flow.progress.botId}
        onBack={(missing) => {
          if (missing) flow.updateProgress({ botId: null });
          setView("source");
        }}
      />
    );
  if (view === "cap" && sourceReady)
    return <SpendingCapStep onBack={() => setView("channel")} />;
  if (view === "channel" && sourceReady)
    return (
      <ChannelStep
        channel={flow.progress.channel}
        referral={referral}
        onSelect={(channel) => flow.updateProgress({ channel })}
        onBack={() => setView("source")}
        onSpendingCap={() => setView("cap")}
        onConnected={(bot) => {
          flow.updateProgress({ botId: bot.id });
          setView("link");
        }}
      />
    );
  return (
    <DataSourceStep
      accountName={accountName}
      ready={sourceReady}
      connecting={flow.connectGoogle.isPending}
      disabled={loading || loadError || (!sourceReady && !flow.googleAvailable)}
      onConnect={() =>
        sourceReady
          ? setView(flow.progress.botId ? "link" : "channel")
          : flow.connectGoogle.mutate()
      }
      onBack={() => setView("account")}
    >
      {loading && <OnboardingNotice>{t("loading")}</OnboardingNotice>}
      {loadError && (
        <OnboardingNotice error>
          {t("loadError")}{" "}
          <Button
            onClick={() => {
              void flow.keys.refetch();
              void flow.catalog.refetch();
              const error = flow.authorization.error;
              if (error && "status" in error && error.status === 404) {
                flow.updateProgress({ googleKeyId: null });
              } else if (flow.progress.googleKeyId) {
                void flow.authorization.refetch();
              }
            }}
          >
            {t("retry")}
          </Button>
        </OnboardingNotice>
      )}
      {sourceReady && (
        <p className="nb-success" role="status">
          <Check size={16} />
          {t("googleConnected")}
        </p>
      )}
      {!loading && !loadError && !sourceReady && !flow.googleAvailable && (
        <OnboardingNotice>{t("googleUnavailable")}</OnboardingNotice>
      )}
      {!sourceReady && callbackFailed && (
        <OnboardingNotice error>{t("googleCancelled")}</OnboardingNotice>
      )}
      {!sourceReady && flow.authorization.data?.status === "active" && (
        <OnboardingNotice error>{t("googleIncomplete")}</OnboardingNotice>
      )}
      {flow.connectGoogle.isError && (
        <OnboardingNotice error>{t("googleCancelled")}</OnboardingNotice>
      )}
    </DataSourceStep>
  );
}

export function NyxbotOnboardingPage() {
  const user = useAuthStore((s) => s.user);
  const authenticated = useAuthStore((s) => s.isAuthenticated);
  const search = nyxbotSearchSchema.parse(
    Object.fromEntries(new URLSearchParams(window.location.search)),
  );
  const returnUrl = new URL("/onboarding", window.location.origin);
  if (search.channel) returnUrl.searchParams.set("channel", search.channel);
  return (
    <I18nextProvider i18n={nyxbotI18n}>
      {authenticated && user ? (
        <ConnectedOnboarding
          key={user.id}
          userId={user.id}
          accountName={user.display_name?.trim() || user.email}
          referral={search.channel}
          callbackFailed={search.status === "error"}
        />
      ) : (
        <SignInStep returnTo={returnUrl.href} />
      )}
    </I18nextProvider>
  );
}
