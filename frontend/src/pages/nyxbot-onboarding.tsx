import { useState } from "react";
import { I18nextProvider, useTranslation } from "react-i18next";
import { Check } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ApiError } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import { useNyxbotOnboarding } from "@/hooks/use-nyxbot-onboarding";
import {
  nyxbotSearchSchema,
  type NyxbotChannel,
} from "@/schemas/nyxbot-onboarding";
import { nyxbotI18n } from "@/features/nyxbot-onboarding/i18n";
import {
  OnboardingLoading,
  OnboardingNotice,
} from "@/features/nyxbot-onboarding/onboarding-shell";
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
  onBackToAccount,
}: {
  readonly userId: string;
  readonly accountName: string;
  readonly referral?: NyxbotChannel;
  readonly callbackFailed: boolean;
  readonly onBackToAccount: () => void;
}) {
  const { t } = useTranslation();
  const flow = useNyxbotOnboarding(userId, referral);
  const [view, setView] = useState<"source" | "channel" | "cap" | "link">(() =>
    flow.progress.botId ? "link" : "channel",
  );
  const loading =
    flow.keys.isPending ||
    flow.catalog.isPending ||
    (Boolean(flow.progress.googleKeyId) && flow.authorization.isPending);
  const loadError =
    flow.keys.isError || flow.catalog.isError || flow.authorization.isError;
  // A URL or locally remembered step never substitutes for a live authorization.
  const sourceReady = Boolean(flow.connectedKey) && !loading && !loadError;

  // Resolve the destination before rendering a step. Explicit Back navigation
  // and an in-progress Google connection keep their data-source view.
  if (
    loading &&
    !loadError &&
    view !== "source" &&
    !flow.connectGoogle.isPending
  )
    return <OnboardingLoading />;

  if (view === "link" && flow.progress.botId && sourceReady)
    return (
      <LinkChannelStep
        botId={flow.progress.botId}
        registrationId={flow.progress.registrationId}
        onBack={(missing) => {
          if (missing)
            flow.updateProgress({ botId: null, registrationId: null });
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
        onConnected={(registration) => {
          flow.updateProgress({
            botId: registration.nyx_channel_bot_id,
            registrationId: registration.registration_id,
          });
          setView("link");
        }}
      />
    );
  return (
    <DataSourceStep
      accountName={accountName}
      ready={sourceReady}
      connecting={flow.connectGoogle.isPending}
      disabled={
        loading ||
        loadError ||
        flow.connectGoogle.isPending ||
        (!sourceReady && !flow.googleAvailable)
      }
      onConnect={() => {
        setView(flow.progress.botId ? "link" : "channel");
        if (!sourceReady) flow.connectGoogle.mutate();
      }}
      onBack={onBackToAccount}
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
        <OnboardingNotice error>
          {t("googleConnectFailed")}
          {flow.connectGoogle.error instanceof ApiError && (
            <p>{flow.connectGoogle.error.message}</p>
          )}
        </OnboardingNotice>
      )}
    </DataSourceStep>
  );
}

export function NyxbotOnboardingPage() {
  const user = useAuthStore((s) => s.user);
  const authenticated = useAuthStore((s) => s.isAuthenticated);
  const authLoading = useAuthStore((s) => s.isLoading);
  const search = nyxbotSearchSchema.parse(
    Object.fromEntries(new URLSearchParams(window.location.search)),
  );
  const callbackStatus = search.provider_status ?? search.status;
  // Return hints select a view; authentication and data grants are still checked.
  const [accountConfirmed, setAccountConfirmed] = useState(
    () => search.step === "source" || Boolean(callbackStatus),
  );
  const returnUrl = new URL("/onboarding", window.location.origin);
  returnUrl.searchParams.set("step", "source");
  if (search.channel) returnUrl.searchParams.set("channel", search.channel);
  return (
    <I18nextProvider i18n={nyxbotI18n}>
      {authLoading && !authenticated ? (
        <OnboardingLoading />
      ) : authenticated && user && accountConfirmed ? (
        <ConnectedOnboarding
          key={user.id}
          userId={user.id}
          accountName={user.display_name?.trim() || user.email}
          referral={search.channel}
          callbackFailed={callbackStatus === "error"}
          onBackToAccount={() => setAccountConfirmed(false)}
        />
      ) : (
        <SignInStep
          returnTo={returnUrl.href}
          accountName={
            authenticated && user
              ? user.display_name?.trim() || user.email
              : undefined
          }
          onContinue={
            authenticated && user ? () => setAccountConfirmed(true) : undefined
          }
        />
      )}
    </I18nextProvider>
  );
}
