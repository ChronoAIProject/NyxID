import { useState } from "react";
import { Check, ChevronDown, ExternalLink, RefreshCw } from "lucide-react";
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
  const [previousRequestId, setPreviousRequestId] = useState<string | null>(
    null,
  );
  if (request && request.id !== previousRequestId)
    setPreviousRequestId(request.id);
  const interrupted = Boolean(
    previousRequestId &&
    !request &&
    !configuration.isPending &&
    !configuration.isError &&
    !cancel.isPending &&
    !cancel.isSuccess &&
    !connect.isPending &&
    !connect.isSuccess,
  );
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
    setPreviousRequestId(null);
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

  const step = !request
    ? 0
    : request.status === "ready" || request.status === "provisioning"
      ? 3
      : request.status === "waiting_consent"
        ? 2
        : 1;
  const steps = [
    {
      title: "Save your bot details",
      location: "In NyxID",
      instructions: (
        <p>
          Choose a label and the NyxID account above, then tap{" "}
          <strong>Save and continue</strong>. Next, you will open Telegram to
          create your bot.
        </p>
      ),
    },
    {
      title: "Create your bot in Telegram",
      location: "In Telegram",
      instructions: (
        <>
          <p>
            You will open{" "}
            <strong>
              {configuration.data.manager_username
                ? `@${configuration.data.manager_username}`
                : "the NyxID setup bot"}
            </strong>
            . This is the setup chat that helps you create your own bot.
          </p>
          <ul className="list-disc space-y-2 pl-5">
            <li>
              Tap <strong>Start</strong> if Telegram asks, then tap{" "}
              <strong>Create bot</strong>.
            </li>
            <li>
              Choose your bot's name and a username ending in{" "}
              <strong>bot</strong>, such as <strong>support_team_bot</strong>.
              Telegram will check whether the username is available.
            </li>
            <li>
              Finish the form, then stay in this setup chat. You will be asked
              to approve the connection next.
            </li>
          </ul>
        </>
      ),
    },
    {
      title: "Approve in Telegram",
      location: "In Telegram",
      instructions: (
        <>
          <p>
            The setup chat will show your new bot and the NyxID account you
            chose. Check the bot's @username, account, and website.
          </p>
          <p>
            If they match your setup, tap <strong>Approve this bot</strong>.
            This lets NyxID receive messages sent to your bot and send replies
            through it.
          </p>
          <p>You will finish connecting the bot on this NyxID page next.</p>
        </>
      ),
    },
    {
      title: "Finish in NyxID",
      location: "In NyxID",
      instructions: (
        <>
          <p>
            Tap <strong>Return to NyxID</strong> in the setup chat, or switch
            back to this page. The <strong>Connect bot</strong> button appears
            here after you approve in Telegram.
          </p>
          <p>
            Tap <strong>Connect bot</strong> to finish. NyxID will then open
            your bot's settings.
          </p>
          <p>
            Choose an AI agent to handle the replies. Then open your new bot's
            chat in Telegram and send a test message.
          </p>
        </>
      ),
    },
  ];
  const telegramAction = launchUrl ? (
    <Button type="button" variant="primary" asChild>
      <a
        href={launchUrl}
        target="_blank"
        rel="noopener noreferrer"
        referrerPolicy="no-referrer"
        onClick={(event) => {
          if (pending) event.preventDefault();
        }}
        aria-disabled={pending}
      >
        <ExternalLink className="size-4" />
        Open Telegram
      </a>
    </Button>
  ) : (
    <Button
      type="button"
      variant="primary"
      disabled={
        pending || (!request && (!label.trim() || label.trim().length > 128))
      }
      onClick={() => void prepare()}
    >
      {pending
        ? "Preparing…"
        : request
          ? "Get Telegram link"
          : "Save and continue"}
    </Button>
  );

  return (
    <div className="space-y-5 rounded-xl border border-border bg-card p-4 sm:p-5">
      <div className="space-y-2">
        <p className="text-sm font-medium">What happens next</p>
        <p className="text-sm text-muted-foreground">
          You will create your bot in Telegram, approve its connection, then
          return here to finish. NyxID handles the bot token for you, so there
          is nothing to copy from BotFather.
        </p>
      </div>
      {error && <ErrorBanner message={error.message} />}
      {interrupted && (
        <p
          role="status"
          className="rounded-lg border border-border p-3 text-sm"
        >
          This setup is no longer active. It may have expired or been completed
          on another page. Check Channel Bots before starting again. If you
          already created a bot, open the help below to see how to reconnect it.
        </p>
      )}
      {request && (
        <div className="space-y-2 rounded-lg bg-muted/40 p-3 text-sm">
          <p className="break-words font-medium">Setting up: {request.label}</p>
          <p className="text-xs text-muted-foreground">
            {request.status === "provisioning" ? (
              "Your bot is saved, but the connection still needs to finish. You can retry below."
            ) : (
              <>
                Setup saved until{" "}
                <time dateTime={request.expires_at}>
                  {new Date(request.expires_at).toLocaleTimeString([], {
                    hour: "numeric",
                    minute: "2-digit",
                  })}
                </time>
                . You can switch to Telegram and come back to this page.
              </>
            )}
          </p>
          <p className="text-xs text-muted-foreground">
            You can leave this page to use Telegram. To come back, open Channel
            Bots → Resume Telegram setup.
          </p>
          {request.owner_user_id !== (orgId ?? actor) && (
            <p className="text-xs">
              This setup uses the account you selected earlier. Cancel setup to
              choose a different account.
            </p>
          )}
          <details className="text-xs text-muted-foreground">
            <summary className="cursor-pointer">Account reference</summary>
            <p className="mt-2 break-all">
              NyxID account ID: {request.owner_user_id}
            </p>
          </details>
        </div>
      )}
      <p className="text-sm font-medium" aria-live="polite">
        Step {step + 1} of {steps.length}
        <span className="ml-2 font-normal text-muted-foreground">
          Follow the steps below to finish setup.
        </span>
      </p>
      <ol aria-label="Telegram setup steps">
        {steps.map(({ title, location, instructions }, index) => (
          <li
            key={title}
            aria-current={step === index ? "step" : undefined}
            className="relative grid grid-cols-[2rem_minmax(0,1fr)] gap-x-3 pb-4 last:pb-0"
          >
            {index < steps.length - 1 && (
              <span
                aria-hidden="true"
                className={`absolute bottom-0 left-[15px] top-8 w-0.5 ${index < step ? "bg-primary/80" : "bg-muted-foreground/60"}`}
              />
            )}
            <span
              aria-hidden="true"
              className={`relative z-10 flex size-8 items-center justify-center rounded-full border text-sm font-semibold ${index === step ? "border-primary bg-primary text-primary-foreground" : index < step ? "border-primary bg-primary text-white" : "border-muted-foreground/40 bg-card text-muted-foreground"}`}
            >
              {index < step ? (
                <Check className="size-4" strokeWidth={3} />
              ) : (
                index + 1
              )}
            </span>
            <details
              open={step === index}
              className={`group min-w-0 rounded-lg border p-3 ${step === index ? "border-primary/40 bg-primary/5" : "border-border"}`}
            >
              <summary className="flex cursor-pointer items-center gap-3 text-sm font-medium">
                <span className="min-w-0 flex-1 space-y-1">
                  <span className="block">
                    <span className="sr-only">Step {index + 1}: </span>
                    {title}
                  </span>
                  <span className="flex items-center gap-1 text-xs font-normal text-muted-foreground">
                    {location} ·{" "}
                    {index < step
                      ? "Complete"
                      : index === step
                        ? "Current step"
                        : index === step + 1
                          ? "Up next"
                          : "Later"}
                  </span>
                </span>
                <ChevronDown className="size-4 shrink-0 text-muted-foreground transition-transform group-open:rotate-180" />
              </summary>
              <div className="mt-3 space-y-3">
                <div className="space-y-3 text-sm text-muted-foreground">
                  {instructions}
                </div>
                {index === step && (
                  <>
                    <p role="status" className="text-sm">
                      {!request
                        ? "Save your details first. You will create the bot in Telegram in the next step."
                        : request.status === "provisioning"
                          ? `The connection for @${request.bot_username} still needs to finish. Tap Retry connection to try again with the same bot.`
                          : step === 3
                            ? `You approved @${request.bot_username} in Telegram. Tap Connect bot to finish.`
                            : step === 2
                              ? "Your bot has been created. Open the setup chat and tap Approve this bot."
                              : request.status === "waiting_bot"
                                ? "You have started the setup chat. Tap Create bot there to make your bot."
                                : launchUrl
                                  ? "Tap Open Telegram below. You will return to this page after approving your bot."
                                  : "Your setup is saved. Tap Get Telegram link, then tap Open Telegram when it appears."}
                    </p>
                    {step === 3 ? (
                      <Button
                        type="button"
                        variant="primary"
                        disabled={pending}
                        onClick={() => void confirm()}
                      >
                        {connect.isPending
                          ? "Connecting…"
                          : request?.status === "provisioning"
                            ? "Retry connection"
                            : "Connect bot"}
                      </Button>
                    ) : (
                      telegramAction
                    )}
                  </>
                )}
              </div>
            </details>
          </li>
        ))}
      </ol>
      {request && (
        <div className="space-y-2">
          <p className="text-xs text-muted-foreground">
            Back from Telegram? This page checks your progress automatically. If
            the step has not changed, tap Check progress.
          </p>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={configuration.isFetching || pending}
            onClick={() => void configuration.refetch()}
          >
            <RefreshCw className="size-4" />
            {configuration.isFetching ? "Checking progress…" : "Check progress"}
          </Button>
        </div>
      )}
      <details className="text-sm">
        <summary className="cursor-pointer text-muted-foreground">
          Need help or returning to an unfinished setup?
        </summary>
        <div className="mt-3 space-y-3 text-sm text-muted-foreground">
          <p>
            <strong>Using one phone?</strong> Switch to the Telegram setup chat
            to create and approve your bot. Then tap Return to NyxID. If you
            closed this page, sign in to the same NyxID account and choose
            Channel Bots → Resume Telegram setup.
          </p>
          <p>
            <strong>Cannot see Create bot?</strong> Check that you are in the
            setup chat named in step 2. Update Telegram if needed, then return
            here and open the setup chat again.
          </p>
          <p>
            <strong>Ran out of time?</strong> You have 15 minutes to finish
            setup. If the time runs out, a bot you created still exists in
            Telegram, but it may not be connected to NyxID.
          </p>
          <p>
            <strong>Already created a bot during this setup?</strong> For up to
            60 minutes after creation, you can try to connect it again:
          </p>
          <ol className="list-decimal space-y-2 pl-5">
            <li>
              Start a new Telegram setup in NyxID and open the setup chat.
            </li>
            <li>
              Send{" "}
              <code className="break-words">/recover @YourBotUsername</code>,
              using the username of the bot you created.
            </li>
            <li>
              If NyxID finds your bot, approve it in Telegram. Then return here
              and tap Connect bot.
            </li>
          </ol>
          <p>
            <strong>Already have a bot from BotFather?</strong> Go back to Add
            Channel Bot and choose Telegram bot token to connect it using its
            existing token.
          </p>
        </div>
      </details>
      {request && request.status !== "provisioning" && (
        <Button
          type="button"
          variant="ghost"
          size="sm"
          disabled={pending}
          onClick={() => {
            void cancel
              .mutateAsync(request.id)
              .then(() => {
                setHandoff(null);
                setPreviousRequestId(null);
              })
              .catch(() => {});
          }}
        >
          Cancel setup
        </Button>
      )}
    </div>
  );
}
