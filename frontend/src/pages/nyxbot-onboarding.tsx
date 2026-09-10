import { useCallback, useEffect, useState } from "react";
import { useNavigate, useSearch } from "@tanstack/react-router";
import { I18nextProvider, useTranslation } from "react-i18next";
import { Check } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ApiError } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import { useNyxbotOnboarding } from "@/hooks/use-nyxbot-onboarding";
import {
  nyxbotSearchSchema,
  readNyxbotProgress,
  type NyxbotChannel,
  type NyxbotStep,
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

type StepNavigation = (
  step: NyxbotStep,
  options?: { replace?: boolean; callbackStatus?: "success" | "error" },
) => void;

function StepRedirect({
  step,
  onNavigate,
  callbackStatus,
}: {
  readonly step: NyxbotStep;
  readonly onNavigate: StepNavigation;
  readonly callbackStatus?: "success" | "error";
}) {
  useEffect(() => {
    onNavigate(step, { replace: true, callbackStatus });
  }, [step, onNavigate, callbackStatus]);
  return <OnboardingLoading />;
}

function ConnectedOnboarding({
  userId,
  accountName,
  referral,
  step,
  callbackStatus,
  onNavigate,
}: {
  readonly userId: string;
  readonly accountName: string;
  readonly referral?: NyxbotChannel;
  readonly step: Exclude<NyxbotStep, "account">;
  readonly callbackStatus?: "success" | "error";
  readonly onNavigate: StepNavigation;
}) {
  const { t } = useTranslation();
  const flow = useNyxbotOnboarding(userId, referral);
  const [showSpendingCap, setShowSpendingCap] = useState(false);
  const loading =
    flow.keys.isPending ||
    flow.catalog.isPending ||
    (Boolean(flow.progress.googleKeyId) && flow.authorization.isPending);
  const loadError =
    flow.keys.isError || flow.catalog.isError || flow.authorization.isError;
  // A URL or locally remembered step never substitutes for a live authorization.
  const sourceReady = Boolean(flow.connectedKey) && !loading && !loadError;

  if (loading && !loadError && !flow.connectGoogle.isPending)
    return <OnboardingLoading />;

  if (step !== "source" && !sourceReady)
    return (
      <StepRedirect
        step="source"
        onNavigate={onNavigate}
        callbackStatus={callbackStatus}
      />
    );
  if (step === "link" && !flow.progress.botId)
    return <StepRedirect step="channel" onNavigate={onNavigate} />;
  // Legacy OAuth callbacks may still target source. An explicit source URL
  // without a callback stays on that step, including after refresh or Back.
  if (step === "source" && sourceReady && callbackStatus)
    return (
      <StepRedirect
        step={flow.progress.botId ? "link" : "channel"}
        onNavigate={onNavigate}
      />
    );
  if (step !== "source" && callbackStatus)
    return <StepRedirect step={step} onNavigate={onNavigate} />;

  if (step === "link" && flow.progress.botId)
    return (
      <LinkChannelStep
        botId={flow.progress.botId}
        registrationId={flow.progress.registrationId}
        onBack={(missing) => {
          if (missing)
            flow.updateProgress({ botId: null, registrationId: null });
          onNavigate("channel");
        }}
      />
    );
  if (step === "channel" && showSpendingCap)
    return <SpendingCapStep onBack={() => setShowSpendingCap(false)} />;
  if (step === "channel")
    return (
      <ChannelStep
        channel={flow.progress.channel}
        referral={referral}
        onSelect={(channel) => flow.updateProgress({ channel })}
        onBack={() => onNavigate("source")}
        onSpendingCap={() => setShowSpendingCap(true)}
        onConnected={(registration) => {
          flow.updateProgress({
            botId: registration.nyx_channel_bot_id,
            registrationId: registration.registration_id,
          });
          onNavigate("link");
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
        if (sourceReady) onNavigate(flow.progress.botId ? "link" : "channel");
        else flow.connectGoogle.mutate();
      }}
      onBack={() => onNavigate("account")}
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
      {!sourceReady && callbackStatus === "error" && (
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
  const search = nyxbotSearchSchema.parse(useSearch({ from: "/onboarding" }));
  const navigate = useNavigate({ from: "/onboarding" });
  const callbackStatus = search.provider_status ?? search.status;
  const step = search.step ?? (callbackStatus ? "source" : "account");
  const onNavigate = useCallback<StepNavigation>(
    (nextStep, options) => {
      void navigate({
        to: "/onboarding",
        search: {
          step: nextStep,
          ...(search.channel ? { channel: search.channel } : {}),
          ...(options?.callbackStatus
            ? { provider_status: options.callbackStatus }
            : {}),
        },
        replace: options?.replace ?? false,
      });
    },
    [navigate, search.channel],
  );
  const resumeStep =
    authenticated && user && readNyxbotProgress(user.id).botId
      ? "link"
      : "channel";
  const returnUrl = new URL("/onboarding", window.location.origin);
  returnUrl.searchParams.set("step", resumeStep);
  if (search.channel) returnUrl.searchParams.set("channel", search.channel);
  return (
    <I18nextProvider i18n={nyxbotI18n}>
      {authLoading && !authenticated ? (
        <OnboardingLoading />
      ) : !search.step ? (
        <StepRedirect
          step={step}
          onNavigate={onNavigate}
          callbackStatus={callbackStatus}
        />
      ) : (!authenticated || !user) && step !== "account" ? (
        <StepRedirect step="account" onNavigate={onNavigate} />
      ) : authenticated && user && step !== "account" ? (
        <ConnectedOnboarding
          key={user.id}
          userId={user.id}
          accountName={user.display_name?.trim() || user.email}
          referral={search.channel}
          step={step}
          callbackStatus={callbackStatus}
          onNavigate={onNavigate}
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
            authenticated && user ? () => onNavigate(resumeStep) : undefined
          }
        />
      )}
    </I18nextProvider>
  );
}
