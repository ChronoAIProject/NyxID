import { useEffect, useState } from "react";
import { useNavigate, useParams } from "@tanstack/react-router";
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

export function ChannelConnectLinkPage() {
  const { token } = useParams({ strict: false }) as { token: string };
  const { user, isAuthenticated, isLoading } = useAuthStore();
  const navigate = useNavigate();
  const link = useChannelConnectLink(
    token,
    isAuthenticated ? user?.id : undefined,
  );
  useEffect(() => {
    if (isLoading || isAuthenticated) return;
    void navigate({
      to: "/login",
      search: {
        return_to: `${window.location.origin}/connect/bot/${encodeURIComponent(token)}`,
      },
    });
  }, [isLoading, isAuthenticated, navigate, token]);
  const data = link.status.data ?? link.preview.data;
  const error = link.status.error ?? link.preview.error;
  return (
    <ChannelConnectionShell
      platform={data?.platform ?? "telegram"}
      platformName={data?.platform}
      complete={data?.status === "completed"}
      email={user?.email}
    >
      <meta name="referrer" content="no-referrer" />
      {error && <ErrorBanner message={error.message} />}
      {link.status.data && user ? (
        <ChannelConnectLinkContext.Provider
          value={{
            token,
            id: link.status.data.id,
            telegramRequestId:
              link.status.data.telegram_request_id ?? undefined,
            connectionId: link.status.data.connection_id ?? undefined,
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
          />
        </ChannelConnectLinkContext.Provider>
      ) : (
        <p role="status" className="text-sm text-muted-foreground">
          Loading connection request…
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
}: {
  initial: ChannelConnectLink;
  current: ChannelConnectLink;
  actor: string;
  action: ReturnType<typeof useChannelConnectLink>["action"];
  refresh: () => Promise<unknown>;
}) {
  // Keep the wizard mounted after completion so its one-time secret survives status refreshes.
  const [showSetup, setShowSetup] = useState(
    initial.status === "pending" && !initial.bot_id,
  );
  const [acknowledged, setAcknowledged] = useState(false);
  const [hasSetupResult, setHasSetupResult] = useState(false);
  const terminal = current.status !== "pending";
  const continueToApp = () => {
    if (current.callback_url) window.location.assign(current.callback_url);
    else setAcknowledged(true);
  };
  return (
    <div className="space-y-4">
      {current.requested_by && (
        <p className="text-xs text-muted-foreground">
          Requested by {current.requested_by}
        </p>
      )}
      {action.error && <ErrorBanner message={action.error.message} />}
      {current.telegram_requires_original_actor && !terminal ? (
        <p role="status" className="text-sm">
          Another administrator started this Telegram setup. Continue using the NyxID account that started it. This page will update when setup finishes.
        </p>
      ) : showSetup &&
      !acknowledged &&
      (hasSetupResult || !["cancelled", "expired"].includes(current.status)) ? (
        <ChannelBotSetup
          open
          onOpenChange={() => {}}
          fullPage
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
          <p role="status" className="text-sm">
            {current.status === "completed"
              ? `${current.label} is connected.`
              : current.status === "pending"
                ? "Your bot was saved. Retry setup to finish connecting it."
                : `This connection request is ${current.status}.`}
          </p>
          {!terminal && (
            <Button
              variant="primary"
              isLoading={action.isPending}
              onClick={() => action.mutate("retry")}
            >
              Retry setup
            </Button>
          )}
          {terminal && current.callback_url && (
            <Button
              variant="primary"
              className="w-full"
              onClick={continueToApp}
            >
              Continue
            </Button>
          )}
          {current.bot_id && (
            <Button
              variant="outline"
              onClick={() =>
                window.location.assign(
                  `/channel-bots/${encodeURIComponent(current.bot_id!)}`,
                )
              }
            >
              Open channel bot
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
          Retry saved setup
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
            Decline request
          </Button>
        )}
    </div>
  );
}
