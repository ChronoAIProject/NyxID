import { useState } from "react";
import { ExternalLink } from "lucide-react";
import { useTelegramNew } from "@/hooks/use-telegram-new";
import { useAuthStore } from "@/stores/auth-store";
import { Button } from "@/components/ui/button";
import { ErrorBanner } from "@/components/shared/error-banner";

export function TelegramNew({
  label,
  orgId,
  onConnected,
}: {
  readonly label: string;
  readonly orgId: string | null;
  readonly onConnected: (id: string) => void;
}) {
  const { configuration, begin, launch, cancel, connect } = useTelegramNew();
  const actor = useAuthStore((state) => state.user?.id);
  const [handoff, setHandoff] = useState<{
    actor: string;
    url: string;
    requestId: string;
  } | null>(null);
  const request = configuration.data?.request;
  const error =
    begin.error ??
    launch.error ??
    connect.error ??
    cancel.error ??
    configuration.error;
  const pending =
    begin.isPending ||
    launch.isPending ||
    connect.isPending ||
    cancel.isPending;
  const launchUrl =
    handoff && handoff.actor === actor && handoff.requestId === request?.id
      ? handoff.url
      : undefined;

  async function prepare() {
    begin.reset();
    launch.reset();
    connect.reset();
    cancel.reset();
    try {
      const result = request
        ? await launch.mutateAsync(request.id)
        : await begin.mutateAsync({ label, target_org_id: orgId ?? undefined });
      if (actor)
        setHandoff({
          actor,
          url: result.launch_url,
          requestId: result.request.id,
        });
    } catch {
      /* Mutation errors are rendered below. */
    }
  }

  async function confirm() {
    if (!request) return;
    try {
      const result = await connect.mutateAsync(request);
      if (result.status === "connected" && result.channel_bot_id)
        onConnected(result.channel_bot_id);
    } catch {
      /* The saved request remains available for retry. */
    }
  }

  if (configuration.isPending)
    return (
      <p role="status" className="text-sm text-muted-foreground">
        Checking Telegram setup…
      </p>
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

  return (
    <div className="space-y-4 rounded-lg border border-border p-4">
      <div className="space-y-1">
        <p className="text-sm font-medium">Create your bot in Telegram</p>
        <p className="text-xs text-muted-foreground">
          Choose a bot name and username in Telegram, approve its connection,
          then return here. Your bot token is handled automatically.
        </p>
      </div>
      {error && <ErrorBanner message={error.message} />}
      {request && (
        <div className="space-y-1 text-xs text-muted-foreground">
          <p>
            Creating:{" "}
            <span className="font-medium text-foreground">{request.label}</span>
          </p>
          <p>Destination account: {request.owner_user_id}</p>
          {request.owner_user_id !== (orgId ?? actor) && (
            <p>
              This existing request belongs to the destination shown above.
              Cancel it to use your newly selected scope.
            </p>
          )}
        </div>
      )}
      {request?.status === "ready" || request?.status === "provisioning" ? (
        <div className="space-y-3">
          <p className="text-sm">
            Connect <strong>@{request.bot_username}</strong> to the destination
            above.
          </p>
          <Button
            type="button"
            variant="primary"
            disabled={pending}
            onClick={() => void confirm()}
          >
            {connect.isPending
              ? "Connecting…"
              : request.status === "provisioning"
                ? "Retry connection"
                : "Connect bot"}
          </Button>
        </div>
      ) : (
        <div className="space-y-3">
          <p role="status" className="text-xs text-muted-foreground">
            {request?.status === "waiting_consent"
              ? "Approve the specific bot in the Telegram setup chat, then return here."
              : request?.status === "waiting_bot"
                ? "Tap Create bot in the Telegram setup chat and choose its name and username."
                : "Prepare your creation request, then open Telegram."}
          </p>
          {launchUrl ? (
            <Button type="button" variant="primary" asChild>
              <a
                href={launchUrl}
                target="_blank"
                rel="noopener noreferrer"
                referrerPolicy="no-referrer"
              >
                <ExternalLink className="size-4" />
                Open Telegram
              </a>
            </Button>
          ) : (
            <Button
              type="button"
              variant="primary"
              disabled={pending || (!request && !label.trim())}
              onClick={() => void prepare()}
            >
              {pending
                ? "Preparing…"
                : request
                  ? "Continue in Telegram"
                  : "Prepare Telegram bot"}
            </Button>
          )}
          <p className="text-xs text-muted-foreground">
            You can reopen Add Channel Bot → Telegram to continue this
            request for 15 minutes. If it expires, a bot you created may still
            exist in Telegram without being connected. For 60 minutes after
            creation, you can start another request and send /recover
            @YourBotUsername in the setup chat.
          </p>
        </div>
      )}
      {request && request.status !== "provisioning" && (
        <Button
          type="button"
          variant="ghost"
          size="sm"
          disabled={pending}
          onClick={() => {
            void cancel
              .mutateAsync(request.id)
              .then(() => setHandoff(null))
              .catch(() => {});
          }}
        >
          Cancel creation request
        </Button>
      )}
    </div>
  );
}
