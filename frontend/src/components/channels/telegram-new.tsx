import { useEffect, useRef, useState } from "react";
import { Check, ExternalLink, Loader2 } from "lucide-react";
import { useTelegramNew } from "@/hooks/use-telegram-new";
import { useAuthStore } from "@/stores/auth-store";
import { Button } from "@/components/ui/button";
import { ErrorBanner } from "@/components/shared/error-banner";

export function TelegramNew({
  label,
  orgId,
  requestId,
  fullPage = false,
  onStarted,
  onCancelled,
  onConnected,
}: {
  readonly label: string;
  readonly orgId: string | null;
  readonly requestId?: string;
  readonly fullPage?: boolean;
  readonly onStarted?: (id: string) => void | Promise<void>;
  readonly onCancelled?: () => void | Promise<void>;
  readonly onConnected: (id: string) => void;
}) {
  const { configuration, begin, launch, cancel, connect } =
    useTelegramNew(requestId);
  const actor = useAuthStore((state) => state.user?.id);
  const active = useRef(true);
  const currentActor = useRef(actor);
  const completed = useRef<string | null>(null);
  const [handoff, setHandoff] = useState<{
    actor: string;
    id: string;
    url: string;
  } | null>(null);
  const request = configuration.data?.request;
  const terminal = Boolean(
    request && ["cancelled", "expired", "suspended"].includes(request.status),
  );
  const preparing = begin.isPending || launch.isPending;
  const pending = preparing || cancel.isPending || connect.isPending;
  const error =
    begin.error ??
    launch.error ??
    cancel.error ??
    connect.error ??
    configuration.error;
  const canLaunch =
    !request ||
    request.status === "cancelled" ||
    request.status === "expired" ||
    ["waiting_telegram", "waiting_bot", "waiting_consent"].includes(
      request.status,
    );
  const launchUrl =
    handoff && handoff.actor === actor && handoff.id === request?.id
      ? handoff.url
      : undefined;

  useEffect(() => {
    currentActor.current = actor;
    active.current = true;
    return () => {
      active.current = false;
    };
  }, [actor]);

  useEffect(() => {
    if (
      request?.status === "connected" &&
      request.channel_bot_id &&
      completed.current !== request.id
    ) {
      completed.current = request.id;
      onConnected(request.channel_bot_id);
    }
  }, [request, onConnected]);

  async function prepare() {
    // Opening the tab in the click preserves browser popup permission across the API call.
    const tab = window.open("about:blank", "_blank");
    if (tab) tab.opener = null;
    begin.reset();
    launch.reset();
    cancel.reset();
    connect.reset();
    try {
      const result =
        request && !terminal
          ? await launch.mutateAsync(request.id)
          : await begin.mutateAsync({
              label,
              target_org_id: orgId ?? undefined,
              auto_connect: true,
            });
      if (!active.current || !actor || currentActor.current !== actor) {
        tab?.close();
        return;
      }
      setHandoff({ actor, id: result.request.id, url: result.launch_url });
      await onStarted?.(result.request.id);
      if (!active.current || currentActor.current !== actor) {
        tab?.close();
        return;
      }
      if (tab && !tab.closed) tab.location.replace(result.launch_url);
    } catch {
      tab?.close();
    }
  }

  async function finishLegacyRequest() {
    if (!request) return;
    try {
      const result = await connect.mutateAsync(request);
      if (
        active.current &&
        currentActor.current === actor &&
        result.status === "connected" &&
        result.channel_bot_id
      )
        onConnected(result.channel_bot_id);
    } catch {
      /* The mutation error is displayed below. */
    }
  }

  if (configuration.isPending)
    return (
      <p role="status" className="text-sm text-muted-foreground">
        Checking Telegram setup…
      </p>
    );
  if (configuration.error && !configuration.data)
    return (
      <div className="space-y-3">
        <ErrorBanner message={configuration.error.message} />
        <Button
          type="button"
          variant="outline"
          onClick={() => void configuration.refetch()}
        >
          Check progress
        </Button>
      </div>
    );
  if (!configuration.data?.available)
    return (
      <div className="space-y-3">
        {error && <ErrorBanner message={error.message} />}
        <p className="text-sm text-muted-foreground">
          An administrator needs to configure a manager bot in Admin → Platform
          Credentials → Telegram — bot creation. You can use Telegram bot token
          to connect an existing bot.
        </p>
        <Button
          type="button"
          variant="outline"
          onClick={() => void configuration.refetch()}
        >
          Check configuration
        </Button>
      </div>
    );

  const creating = Boolean(request && !terminal);
  const connecting =
    request?.status === "ready" || request?.status === "provisioning";
  const connected = request?.status === "connected";
  return (
    <div className={fullPage ? "space-y-4 break-words" : "space-y-4 break-words rounded-xl border border-border bg-card p-4 sm:p-5"}>
      {error && <ErrorBanner message={error.message} />}
      {(connecting && request?.auto_connect) || connected ? (
        <div className="space-y-3">
          <p role="status" className="flex items-center gap-2 text-sm">
            {connected ? <Check className="size-3 text-success" /> : <Loader2 className="size-3 animate-spin" />}
            {connected ? `@${request?.bot_username} is connected.` : `Connecting @${request?.bot_username}…`}
          </p>
          <p className="text-xs text-muted-foreground">
            You can close this page. NyxID will finish connecting your bot and send a message in the Telegram setup chat.
          </p>
          {request?.connection_error && <p role="status" className="text-xs text-muted-foreground">{request.connection_error}</p>}
        </div>
      ) : fullPage ? (
        <div className="space-y-4">
          <p className="text-xs leading-relaxed text-muted-foreground">
            Choose your bot’s name and username in Telegram. NyxID connects it automatically when you finish.
          </p>
          {canLaunch && (
            <Button
              type="button"
              variant="primary"
              className="w-full"
              isLoading={preparing}
              disabled={pending || ((!request || terminal) && !label.trim())}
              onClick={() => void prepare()}
            >
              {request && !terminal ? "Reopen Telegram" : "Continue in Telegram"}
              <ExternalLink className="size-3" aria-hidden="true" />
            </Button>
          )}
          {launchUrl && canLaunch && (
            <p className="text-xs text-muted-foreground">
              Telegram didn’t open?{" "}
              <a className="ph-no-capture underline" href={launchUrl} target="_blank" rel="noopener noreferrer" referrerPolicy="no-referrer">Open Telegram</a>
            </p>
          )}
          {creating && (
            <p role="status" className="flex items-center gap-2 text-xs text-muted-foreground">
              <Loader2 className="size-3 shrink-0 animate-spin" aria-hidden="true" />
              {connecting ? `Connecting @${request?.bot_username}…` : "Waiting for you to create your bot in Telegram…"}
            </p>
          )}
          {connecting && request?.connection_error && (
            <p role="status" className="text-xs text-muted-foreground">{request.connection_error}</p>
          )}
        </div>
      ) : <ol aria-label="Telegram setup steps" className="space-y-4">
        <li className="flex gap-3">
          <span
            className="flex size-7 shrink-0 items-center justify-center rounded-full border border-border text-xs"
            aria-hidden="true"
          >
            {creating ? <Check className="size-3 text-success" /> : "1"}
          </span>
          <div className="space-y-2">
            <p className="text-sm font-medium">Continue in Telegram</p>
            <p className="text-xs text-muted-foreground">
              Create a new bot for the account selected above. NyxID will
              connect it automatically when you finish Telegram’s form.
            </p>
            {canLaunch && (
              <Button
                type="button"
                variant="primary"
                isLoading={preparing}
                disabled={pending || ((!request || terminal) && !label.trim())}
                onClick={() => void prepare()}
              >
                <ExternalLink className="size-3" />
                {request && !terminal
                  ? "Reopen Telegram"
                  : "Continue in Telegram"}
              </Button>
            )}
            {launchUrl && canLaunch && (
              <p className="text-xs text-muted-foreground">
                Telegram didn’t open?{" "}
                <a
                  className="ph-no-capture underline"
                  href={launchUrl}
                  target="_blank"
                  rel="noopener noreferrer"
                  referrerPolicy="no-referrer"
                >
                  Open Telegram
                </a>
              </p>
            )}
          </div>
        </li>
        <li
          className="flex gap-3"
          aria-current={creating && !connected ? "step" : undefined}
        >
          <span
            className="flex size-7 shrink-0 items-center justify-center rounded-full border border-border text-xs"
            aria-hidden="true"
          >
            {connected ? <Check className="size-3 text-success" /> : "2"}
          </span>
          <div className="space-y-2">
            <p className="text-sm font-medium">Create your bot</p>
            <p className="text-xs text-muted-foreground">
              In the setup chat, tap <strong>Create bot</strong> and finish
              Telegram’s name and username form. Tap <strong>Start</strong>{" "}
              first if Telegram asks.
            </p>
            <p className="text-xs text-muted-foreground">
              You can stay in Telegram. This page updates automatically when the
              connection is complete.
            </p>
            {creating && (
              <p role="status" className="flex items-center gap-2 text-sm">
                {connected ? (
                  <Check className="size-3 text-success" />
                ) : (
                  <Loader2 className="size-3 animate-spin" />
                )}
                {connected
                  ? `@${request?.bot_username} is connected.`
                  : connecting
                    ? `Connecting @${request?.bot_username}…`
                    : "Waiting for you to create your bot in Telegram…"}
              </p>
            )}
            {connecting && request?.connection_error && (
              <p role="status" className="text-xs text-muted-foreground">
                {request.connection_error}
              </p>
            )}
          </div>
        </li>
      </ol>}
      {request && !request.auto_connect && !terminal && (
        <div className="space-y-2 rounded-lg border border-border p-3 text-xs">
          <p>
            This setup was started with the previous flow. Finish its Telegram
            approval, then complete the saved connection below{fullPage ? "." : ", or cancel and start again."}
          </p>
          {connecting && (
            <Button
              type="button"
              variant="primary"
              disabled={pending}
              onClick={() => void finishLegacyRequest()}
            >
              {connect.isPending ? "Connecting…" : "Finish saved connection"}
            </Button>
          )}
        </div>
      )}
      {terminal && (
        <p role="status" className="text-sm text-muted-foreground">
          {request?.status === "expired"
            ? "This setup expired. Start again to create a new bot."
            : request?.status === "suspended"
              ? "This bot’s management changed. Manage the saved bot from Channel Bots."
              : "Setup cancelled. You can start again."}
        </p>
      )}
      {!fullPage && request && !terminal && !connected && (
        <div className="space-y-2 border-t border-border pt-3">
          <p className="text-xs text-muted-foreground">
            Setting up: {request.label}.{" "}
            {request.status !== "provisioning" && (
              <>
                Saved until{" "}
                <time dateTime={request.expires_at}>
                  {new Date(request.expires_at).toLocaleTimeString([], {
                    hour: "numeric",
                    minute: "2-digit",
                  })}
                </time>
                .
              </>
            )}
          </p>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            disabled={pending}
            onClick={() => void configuration.refetch()}
          >
            Check progress
          </Button>
          {request.status !== "provisioning" && (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              disabled={pending}
              onClick={() => {
                void cancel
                  .mutateAsync(request.id)
                  .then(async () => {
                    if (!active.current || currentActor.current !== actor) return;
                    setHandoff(null);
                    await onCancelled?.();
                  })
                  .catch(() => {});
              }}
            >
              Cancel setup
            </Button>
          )}
        </div>
      )}
    </div>
  );
}
