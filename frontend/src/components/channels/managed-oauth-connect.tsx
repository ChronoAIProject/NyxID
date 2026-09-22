import { useEffect, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { ExternalLink } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { ErrorBanner } from "@/components/shared/error-banner";
import { DetailSection } from "@/components/shared/detail-section";
import { DetailRow } from "@/components/shared/detail-row";
import { XEventsSettings } from "./x-events-settings";
import { useChannelPlatformViews } from "@/hooks/use-channel-platforms";
import {
  openOAuthPopup,
  openOAuthChannel,
  postOAuthAck,
} from "@/lib/oauth-popup";
import { isOAuthResultMessage } from "@/schemas/oauth-popup";
import {
  startManagedOAuth,
  completeManagedOAuth,
  useManagedOnboarding,
} from "@/hooks/use-channel-managed";
import type {
  ManagedFlowProps,
  ManagedDetailProps,
} from "./managed-flow-types";

export function ManagedOAuthConnect({
  platform,
  bootstrap,
  label,
  orgId,
  onConnected,
  botId,
  fullPage = false,
}: ManagedFlowProps) {
  const [stage, setStage] = useState<
    "starting" | "authorizing" | "completing" | null
  >(null);
  const [error, setError] = useState<string | null>(null);
  const [canRetry, setCanRetry] = useState(false);
  const cleanup = useRef<(() => void) | null>(null);
  const queryClient = useQueryClient();
  const { getPlatform } = useChannelPlatformViews();
  const descriptor = getPlatform(platform);
  useEffect(() => () => cleanup.current?.(), []);

  function connect() {
    if (!bootstrap.available || (cleanup.current && !canRetry)) return;
    cleanup.current?.();
    setStage(null);
    setCanRetry(false);
    setError(null);
    const popup = openOAuthPopup();
    if (!popup) {
      setError("Sign-in could not open. Allow popups for this site and retry.");
      return;
    }
    const controller = new AbortController();
    let channel: BroadcastChannel | null = null;
    let active = true;
    let completing = false;
    let closeMonitor: number | undefined;
    const timeout = window.setTimeout(
      () => fail("Account sign-in timed out. Please reconnect."),
      10 * 60_000,
    );
    const stop = () => {
      active = false;
      controller.abort();
      channel?.close();
      popup.close();
      window.clearTimeout(timeout);
      window.clearInterval(closeMonitor);
      cleanup.current = null;
    };
    const fail = (message: string) => {
      if (!active) return;
      stop();
      setCanRetry(false);
      setStage(null);
      setError(message);
    };
    cleanup.current = stop;
    setStage("starting");
    void startManagedOAuth(platform, label, orgId, controller.signal)
      .then(async (started) => {
        if (!active) return;
        channel = openOAuthChannel(started.attempt_nonce);
        if (!channel) {
          fail(
            "This browser cannot complete account sign-in. Try a supported browser.",
          );
          return;
        }
        channel.onmessage = (event: MessageEvent<unknown>) => {
          if (!active || completing || !isOAuthResultMessage(event.data))
            return;
          if (event.data.status === "error") {
            fail(
              "Account authorization failed or was cancelled. Please reconnect.",
            );
            return;
          }
          completing = true;
          setCanRetry(false);
          window.clearInterval(closeMonitor);
          setStage("completing");
          void completeManagedOAuth(
            platform,
            {
              connection_id: started.connection_id,
              label: label.trim(),
              ...(orgId ? { target_org_id: orgId } : {}),
            },
            controller.signal,
            botId,
          )
            .then(async (bot) => {
              if (!active) return;
              if (channel) postOAuthAck(channel);
              stop();
              setStage(null);
              await queryClient.invalidateQueries({
                queryKey: ["channel-bots"],
              });
              onConnected(bot);
            })
            .catch((error: unknown) =>
              fail(
                error instanceof Error
                  ? error.message
                  : "Unable to connect account",
              ),
            );
        };
        setStage("authorizing");
        await popup.navigate(
          started.authorization_url,
          started.attempt_nonce,
          descriptor.label,
        );
        if (active && !completing) {
          closeMonitor = window.setInterval(() => {
            if (popup.isClosed()) {
              // COOP can report a live popup as closed; keep listening until retry.
              setCanRetry(true);
              window.clearInterval(closeMonitor);
            }
          }, 500);
        }
      })
      .catch((error: unknown) =>
        fail(
          error instanceof Error
            ? error.message
            : "Unable to start account sign-in",
        ),
      );
  }

  return (
    <div className="space-y-3">
      {error && <ErrorBanner message={error} />}
      {canRetry && (
        <p role="status" className="text-xs text-muted-foreground">
          Finish authorization in the sign-in window. If you closed it, retry
          the connection.
        </p>
      )}
      <div className="flex flex-wrap gap-2">
        <Button
          type="button"
          variant="primary"
          className={fullPage ? "w-full" : undefined}
          isLoading={stage !== null && !canRetry}
          disabled={
            !bootstrap.available || !label.trim() || label.trim().length > 128
          }
          onClick={connect}
        >
          <ExternalLink className="size-3" />
          {canRetry
            ? "Retry connection"
            : stage === "starting"
              ? "Opening sign-in..."
              : stage === "authorizing"
                ? "Waiting for authorization..."
                : stage === "completing"
                  ? "Connecting account..."
                  : botId
                    ? "Reconnect"
                    : (descriptor.connectLabel ??
                      `Connect ${descriptor.label}`)}
        </Button>
        {stage && !fullPage && (
          <Button
            type="button"
            variant="ghost"
            onClick={() => {
              cleanup.current?.();
              setCanRetry(false);
              setStage(null);
            }}
          >
            Cancel
          </Button>
        )}
      </div>
    </div>
  );
}

export function ManagedOAuthDetail({ bot, orgId }: ManagedDetailProps) {
  const { getPlatform } = useChannelPlatformViews();
  const bootstrap = useManagedOnboarding(bot.platform);
  return (
    <DetailSection title="Connected account">
      <DetailRow label="Handle" value={bot.platform_bot_username} />
      <DetailRow
        label="Connection"
        value={bot.connection_id ?? "Missing"}
        copyable
      />
      <DetailRow
        label="Message delivery"
        value={bot.webhook_registered ? "Real-time webhooks" : "Polling"}
      />
      {!bot.webhook_registered && (
        <>
          <DetailRow
            label="Last polled"
            value={bot.last_polled_at ?? "Not yet polled"}
          />
          <DetailRow label="Next poll" value={bot.next_poll_at ?? "Paused"} />
          <DetailRow
            label="Cursor"
            value={bot.poll_cursor ?? "Not initialized"}
            copyable
          />
          <DetailRow
            label="Consecutive errors"
            value={String(bot.poll_error_count ?? 0)}
          />
        </>
      )}
      <div className="space-y-3 py-3">
        {bot.platform === "x" && <XEventsSettings bot={bot} />}
        {!bot.webhook_registered && bot.last_poll_notice && (
          <p role="status" className="text-xs text-warning">
            {bot.last_poll_notice}
          </p>
        )}
        {bot.error && <ErrorBanner message={bot.error} />}
        {bootstrap.isError && (
          <ErrorBanner
            message="Unable to load account connection settings"
            onRetry={bootstrap.refetch}
          />
        )}
        {bootstrap.data?.available && (
          <ManagedOAuthConnect
            platform={bot.platform}
            bootstrap={bootstrap.data}
            label={bot.label}
            orgId={orgId}
            botId={bot.id}
            onConnected={() => toast.success("Account reconnected")}
          />
        )}
        {bootstrap.data && !bootstrap.data.available && (
          <p className="text-xs text-muted-foreground">
            Account connection is not available until an admin configures{" "}
            {getPlatform(bot.platform).label}.
          </p>
        )}
      </div>
    </DetailSection>
  );
}
