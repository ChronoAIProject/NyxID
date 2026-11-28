import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { useEffect, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { GuestSetupEditor } from "@/features/channel-guest-access/guest-setup";
import { ChannelBotSetup } from "@/components/channels/channel-bot-setup";
import { ChannelConnectionShell } from "@/components/channels/channel-connection-shell";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Button } from "@/components/ui/button";
import {
  ChannelConnectLinkContext,
  useChannelConnectLink,
} from "@/hooks/use-channel-connect-link";
import { useAuthStore } from "@/stores/auth-store";
import type { ChannelConnectLink } from "@/schemas/channel-connect-links";

export function ChannelConnectLinkContent({
  token,
  embedded = false,
  onClose,
}: {
  readonly token: string;
  readonly embedded?: boolean;
  readonly onClose?: () => void;
}) {
  useUiTranslation();
  const { user, isAuthenticated, isLoading } = useAuthStore();
  const navigate = useNavigate();
  const link = useChannelConnectLink(
    token,
    isAuthenticated ? user?.id : undefined,
  );
  useEffect(() => {
    if (embedded || isLoading || isAuthenticated) return;
    void navigate({
      to: "/login",
      search: {
        return_to: `${window.location.origin}/connect/bot/${encodeURIComponent(token)}`,
      },
    });
  }, [embedded, isLoading, isAuthenticated, navigate, token]);
  const data = link.status.data ?? link.preview.data;
  const error = link.status.error ?? link.preview.error;
  return (
    <ChannelConnectionShell
      platform={data?.platform ?? "telegram"}
      platformName={
        data?.platform === "telegram-new" || data?.platform === "telegram"
          ? "Telegram"
          : data?.platform
      }
      complete={data?.status === "completed"}
      email={user?.email}
      embedded={embedded}
    >
      {!embedded && <meta name="referrer" content="no-referrer" />}
      {error && <ErrorBanner message={error.message} />}
      {link.status.data && user ? (
        <ChannelConnectLinkContext.Provider
          value={{
            token,
            id: link.status.data.id,
            telegramRequestId:
              link.status.data.telegram_request_id ?? undefined,
            connectionId: link.status.data.connection_id ?? undefined,
            recoveryUnavailable: link.recoveryUnavailable,
            isRecoveryCurrent: link.isRecoveryCurrent,
            getRecoveryGeneration: link.getRecoveryGeneration,
            refresh: link.refresh,
          }}
        >
          <ConnectSession
            key={`${user.id}:${link.status.data.id}`}
            initial={link.status.data}
            current={link.status.data}
            actor={user.id}
            action={link.action}
            refresh={link.refresh}
            embedded={embedded}
            onClose={onClose}
          />
        </ChannelConnectLinkContext.Provider>
      ) : (
        <p role="status" className="text-14 text-muted-foreground">
          {translateUi("Loading connection request…")}
        </p>
      )}
    </ChannelConnectionShell>
  );
}

function ConnectSession({
  initial,
  current,
  actor,
  action,
  refresh,
  embedded,
  onClose,
}: {
  initial: ChannelConnectLink;
  current: ChannelConnectLink;
  actor: string;
  action: ReturnType<typeof useChannelConnectLink>["action"];
  refresh: () => Promise<unknown>;
  embedded: boolean;
  onClose?: () => void;
}) {
  useUiTranslation();
  // Keep the wizard mounted after completion so its one-time secret survives status refreshes.
  const [showSetup, setShowSetup] = useState(
    initial.status === "pending" && !initial.bot_id,
  );
  const [acknowledged, setAcknowledged] = useState(false);
  const [hasSetupResult, setHasSetupResult] = useState(false);
  const terminal = current.status !== "pending";
  const continueToApp = () => {
    if (embedded) onClose?.();
    else if (current.callback_url) window.location.assign(current.callback_url);
    else setAcknowledged(true);
  };
  return (
    <div className="space-y-4">
      {current.requested_by && (
        <p className="text-13 text-muted-foreground">
          {translateUi("Requested by ")}
          {current.requested_by}
        </p>
      )}
      {action.error && <ErrorBanner message={action.error.message} />}
      {current.telegram_requires_original_actor && !terminal ? (
        <p role="status" className="text-14">
          {translateUi(
            "Another administrator started this Telegram setup. Continue using the NyxID account that started it. This page will update when setup finishes.",
          )}
        </p>
      ) : showSetup &&
        !acknowledged &&
        (hasSetupResult ||
          !["cancelled", "expired"].includes(current.status)) ? (
        <ChannelBotSetup
          open
          onOpenChange={() => {}}
          fullPage
          stayInPlace={embedded}
          defaultPlatform={current.platform}
          defaultLabel={current.label}
          defaultOrgId={current.owner_id === actor ? null : current.owner_id}
          onComplete={() => {
            setHasSetupResult(true);
            void refresh();
          }}
          onContinue={async () => {
            await refresh();
            setShowSetup(false);
          }}
        />
      ) : (
        <div className="space-y-3">
          <p role="status" className="text-14">
            {current.status === "completed"
              ? translateUi("{{value1}} is connected.", {
                  value1: current.label,
                })
              : current.status === "pending"
                ? translateUi(
                    "Your bot was saved. Retry setup to finish connecting it.",
                  )
                : translateUi("This connection request is {{value1}}.", {
                    value1: current.status,
                  })}
          </p>
          {current.bot_id && <GuestSetupEditor botId={current.bot_id} />}
          {!terminal && (
            <Button
              variant="primary"
              isLoading={action.isPending}
              onClick={() => action.mutate("retry")}
            >
              {translateUi("Retry setup")}
            </Button>
          )}
          {terminal && (embedded || current.callback_url) && (
            <Button
              variant="primary"
              className="w-full"
              onClick={continueToApp}
            >
              {embedded ? translateUi("Back to chat") : translateUi("Continue")}
            </Button>
          )}
          {current.bot_id && !embedded && (
            <Button
              variant="outline"
              onClick={() =>
                window.location.assign(
                  `/channel-bots/${encodeURIComponent(current.bot_id!)}`,
                )
              }
            >
              {translateUi("Open channel bot")}
            </Button>
          )}
        </div>
      )}
      {showSetup && !terminal && current.bot_id && current.last_error && (
        <Button
          variant="outline"
          onClick={() => {
            setShowSetup(false);
            action.mutate("retry");
          }}
        >
          {translateUi("Retry saved setup")}
        </Button>
      )}
      {!terminal &&
        !current.bot_id &&
        !current.connection_id &&
        !current.telegram_request_id && (
          <Button
            variant="ghost"
            isLoading={action.isPending}
            onClick={() => action.mutate("decline")}
          >
            {translateUi("Decline request")}
          </Button>
        )}
    </div>
  );
}
