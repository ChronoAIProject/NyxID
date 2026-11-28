import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { getFormattingLocale } from "@/lib/formatting-locale";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Check, ExternalLink, Loader2 } from "lucide-react";
import { useTelegramNew } from "@/hooks/use-telegram-new";
import { useChannelConnectLinkContext } from "@/hooks/use-channel-connect-link";
import { useAuthStore } from "@/stores/auth-store";
import { Button } from "@/components/ui/button";
import { TelegramPromptStatus } from "./telegram-prompt-status";
import { ErrorBanner } from "@/components/shared/error-banner";
import type { TelegramNewRequest } from "@/schemas/telegram-new";

function matchesLiveCreationRequest(
  candidate: TelegramNewRequest | null | undefined,
  captured: TelegramNewRequest,
  checkedAt: number,
): candidate is TelegramNewRequest {
  return Boolean(candidate
    && candidate.id === captured.id
    && candidate.revision === captured.revision
    && candidate.owner_user_id === captured.owner_user_id
    && candidate.status === "waiting_bot"
    && !candidate.setup_notice
    && candidate.can_open_creation_form
    && Date.parse(candidate.expires_at) > checkedAt);
}

function creationUrlForManager(url: string, managerUsername: string | null | undefined) {
  if (!managerUsername || !/^[A-Za-z0-9_]{5,32}$/.test(managerUsername)) return undefined;
  // Issued URLs have already passed telegramCreationLinkSchema. Change only
  // the manager's public username, retaining the chosen bot name and query bytes.
  const current = new URL(url);
  current.pathname = current.pathname.replace(/^\/newbot\/[^/]+\//, `/newbot/${managerUsername}/`);
  return current.toString();
}

export function TelegramNew({
  label,
  orgId,
  requestId,
  fullPage = false,
  stayInPlace = false,
  onStarted,
  onCancelled,
  onConnected,
}: {
  readonly label: string;
  readonly orgId: string | null;
  readonly requestId?: string;
  readonly fullPage?: boolean;
  readonly stayInPlace?: boolean;
  readonly onStarted?: (id: string) => void | Promise<void>;
  readonly onCancelled?: () => void | Promise<void>;
  readonly onConnected: (id: string, username?: string | null) => void;
}) {
  useUiTranslation();
  const { configuration, begin, launch, creationLink, regenerateUsername, cancel, connect, getCurrentConfiguration, getRecoveryGeneration } =
    useTelegramNew(requestId);
  const link = useChannelConnectLinkContext();
  const actor = useAuthStore((state) => state.user?.id);
  const active = useRef(true);
  const currentActor = useRef(actor);
  const recoveryContext = useRef({});
  const completed = useRef<string | null>(null);
  const [handoff, setHandoff] = useState<{
    actor: string;
    id: string;
    url: string;
  } | null>(null);
  const request = configuration.data?.request;
  const managerUsername = configuration.data?.manager_username;
  const recoveryReadsReady = configuration.isSuccess && configuration.data.available && !link?.recoveryUnavailable
    && (!link?.isRecoveryCurrent || Boolean(request && link.isRecoveryCurrent(request.id, configuration.data.hostedRecoveryGeneration)));
  const requestExpiresAt = request?.expires_at;
  const canOpenCreationForm = request?.can_open_creation_form;
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const expiry = requestExpiresAt ? Date.parse(requestExpiresAt) : NaN;
    if (!canOpenCreationForm || !Number.isFinite(expiry)) return;
    const timer = window.setTimeout(() => setNow(Date.now()), Math.min(2_147_483_647, Math.max(0, expiry - Date.now() + 1)));
    return () => window.clearTimeout(timer);
  }, [requestExpiresAt, canOpenCreationForm]);
  const recoveryUnexpired = Boolean(request && Date.parse(request.expires_at) > now);
  const [recovery, setRecovery] = useState<{ actor: string; requestId?: string; linkId?: string; id: string; revision: number; generation: string; username?: string; url: string } | null>(null);
  const currentRecoveryUrl = recovery && recoveryReadsReady && recovery.generation === getRecoveryGeneration()
    ? creationUrlForManager(recovery.url, managerUsername) : undefined;
  const recoveryMatchesRequest = recovery && request && currentRecoveryUrl
    && recovery.actor === actor && recovery.requestId === requestId && recovery.linkId === link?.id
    && recovery.id === request.id && recovery.revision === request.revision
    && (!recovery.username || request.username_suggestions?.includes(recovery.username))
    && request.status === "waiting_bot" && !request.setup_notice && request.can_open_creation_form && recoveryUnexpired;
  // Recovering a read or returning to an earlier context cannot restore an old
  // link. The owner must explicitly request a fresh, validated form URL.
  if (recovery && !recoveryMatchesRequest) setRecovery(null);
  const terminal = Boolean(
    request && ["cancelled", "expired", "suspended"].includes(request.status),
  );
  const preparing = begin.isPending || launch.isPending;
  const pending = preparing || creationLink.isPending || regenerateUsername.isPending || cancel.isPending || connect.isPending;
  const error =
    regenerateUsername.error ??
    creationLink.error ??
    begin.error ??
    launch.error ??
    cancel.error ??
    connect.error ??
    configuration.error;
  const canLaunch =
    !request?.setup_notice &&
    (!request ||
      request.status === "cancelled" ||
      request.status === "expired" ||
      ["waiting_telegram", "waiting_bot", "waiting_consent"].includes(
        request.status,
      ));
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

  useLayoutEffect(() => {
    // Replacing the caller's request or hosted link must invalidate callbacks
    // at commit, before a promise can use the previous query key. Cleanup also
    // invalidates recovery synchronously when the caller unmounts.
    recoveryContext.current = {};
    return () => { recoveryContext.current = {}; };
  }, [actor, requestId, link?.id]);

  useEffect(() => {
    if (
      request?.status === "connected" &&
      request.channel_bot_id &&
      completed.current !== request.id
    ) {
      completed.current = request.id;
      onConnected(request.channel_bot_id, request.bot_username);
    }
  }, [request, onConnected]);

  async function prepare() {
    // Opening the tab in the click preserves browser popup permission across the API call.
    const tab = window.open("about:blank", "_blank");
    if (tab) tab.opener = null;
    creationLink.reset();
    regenerateUsername.reset();
    setRecovery(null);
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

  async function openCreationForm(index = 0) {
    if (!request?.can_open_creation_form || request.setup_notice || request.status !== "waiting_bot"
      || pending || !actor || !recoveryReadsReady || !managerUsername || !(Date.parse(request.expires_at) > Date.now())) return;
    const captured = request;
    const context = recoveryContext.current;
    const generation = getRecoveryGeneration();
    const tab = window.open("about:blank", "_blank");
    if (tab) tab.opener = null;
    creationLink.reset();
    setRecovery(null);
    try {
      const result = await creationLink.mutateAsync(captured.id);
      const latestConfig = getCurrentConfiguration();
      const latest = latestConfig?.request;
      const checkedAt = Date.now();
      if (!active.current || currentActor.current !== actor || recoveryContext.current !== context
        || getRecoveryGeneration() !== generation
        || !matchesLiveCreationRequest(latest, captured, checkedAt)
        || !matchesLiveCreationRequest(result.request, captured, checkedAt)) {
        tab?.close();
        return;
      }
      const choice = result.username_suggestions?.[index];
      const expectedUsername = captured.username_suggestions?.[index];
      if ((index > 0 && !choice) || (choice && expectedUsername && (choice.username !== expectedUsername
        || (latest.username_suggestions && latest.username_suggestions[index] !== expectedUsername)))) {
        tab?.close();
        return;
      }
      const url = creationUrlForManager(choice?.creation_url ?? result.creation_url, latestConfig?.manager_username);
      if (!url) {
        tab?.close();
        return;
      }
      setRecovery({ actor, requestId, linkId: link?.id, id: captured.id, revision: captured.revision, generation, username: choice?.username, url });
      if (tab && !tab.closed) tab.location.replace(url);
    } catch {
      tab?.close();
    }
  }

  async function regenerate() {
    if (!request || pending) return;
    creationLink.reset();
    setRecovery(null);
    try { await regenerateUsername.mutateAsync(request.id); } catch { /* Displayed below. */ }
  }

  async function checkProgress() {
    await configuration.refetch();
    await link?.refresh();
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
        onConnected(result.channel_bot_id, result.bot_username);
    } catch {
      /* The mutation error is displayed below. */
    }
  }

  if (configuration.isPending)
    return (
      <p role="status" className="text-14 text-muted-foreground">{translateUi("Checking Telegram setup…")}</p>
    );
  if (configuration.error && !configuration.data)
    return (
      <div className="space-y-3">
        <ErrorBanner message={configuration.error.message} />
        <Button
          type="button"
          variant="outline"
          onClick={() => void checkProgress()}
        >{translateUi("Check progress")}</Button>
      </div>
    );
  if (!configuration.data?.available && !request?.setup_notice)
    return (
      <div className="space-y-3">
        {error && <ErrorBanner message={error.message} />}
        <p className="text-14 text-muted-foreground">{translateUi("An administrator needs to configure a manager bot in Admin → Platform Credentials → Telegram — bot creation. You can use Telegram bot token to connect an existing bot.")}</p>
        <Button
          type="button"
          variant="outline"
          onClick={() => void configuration.refetch()}
        >{translateUi("Check configuration")}</Button>
      </div>
    );

  const creating = Boolean(request && !terminal);
  const connecting =
    request?.status === "ready" || request?.status === "provisioning";
  const connected = request?.status === "connected";
  return (
    <div className={fullPage ? "space-y-4 break-words" : "space-y-4 break-words rounded-xl border border-border bg-card p-4 sm:p-5"}>
      {error && <ErrorBanner message={error.message} />}
      {link && begin.error && (
        <a href="/channel-bots?connect=telegram-new" target="_blank" rel="noopener noreferrer" className="type-row-title underline underline-offset-4">{translateUi("Open existing Telegram setup")}</a>
      )}
      {request?.setup_notice && (
        <div className="space-y-3 type-secondary text-muted-foreground">
          <p role="status">{request.setup_notice.message}</p>
          {request.connection_error && <p role="status">{request.connection_error}</p>}
          <div className="flex flex-wrap gap-2">
            <Button asChild variant="outline">
              <a href={request.setup_notice.status_url}>{translateUi("Check status")}</a>
            </Button>
            <Button asChild variant="outline">
              <a href={request.setup_notice.new_setup_url}>{translateUi("Start a new setup")}</a>
            </Button>
            {request.setup_notice.agent_url && (
              <Button asChild variant="outline">
                <a href={request.setup_notice.agent_url}>{translateUi("Open the agent")}</a>
              </Button>
            )}
          </div>
        </div>
      )}
      {request?.setup_notice ? null : (connecting && request?.auto_connect) || connected ? (
        <div className="space-y-3">
          <p role="status" className="flex items-center gap-2 text-14">
            {connected ? <Check className="size-3 text-success" /> : <Loader2 className="size-3 animate-spin" />}
            {connected ? translateUi("@{{value1}} is connected.", { value1: request?.bot_username }) : translateUi("Connecting @{{value1}}…", { value1: request?.bot_username })}
          </p>
          <p className="text-13 text-muted-foreground">{translateUi("You can close this ")}{stayInPlace ? translateUi("dialog") : translateUi("page")}{translateUi(". NyxID will finish connecting your bot and send a message in the Telegram setup chat. ")}</p>
          {request?.connection_error && <p role="status" className="text-13 text-muted-foreground">{request.connection_error}</p>}
        </div>
      ) : fullPage ? (
        <div className="space-y-4">
          <p className="text-13 text-muted-foreground">{translateUi("Choose your bot’s name and username in Telegram. NyxID connects it automatically when you finish.")}</p>
          {canLaunch && (
            <Button
              type="button"
              variant="primary"
              className="w-full"
              isLoading={preparing}
              disabled={pending || ((!request || terminal) && !label.trim())}
              onClick={() => void prepare()}
            >
              {request && !terminal ? translateUi("Reopen Telegram") : translateUi("Continue in Telegram")}
              <ExternalLink className="size-3" aria-hidden="true" />
            </Button>
          )}
          {launchUrl && canLaunch && (
            <p className="text-13 text-muted-foreground">{translateUi("Telegram didn’t open?")}{" "}
              <a className="ph-no-capture underline" href={launchUrl} target="_blank" rel="noopener noreferrer" referrerPolicy="no-referrer">{translateUi("Open Telegram")}</a>
            </p>
          )}
          {connecting && (
            <p role="status" className="flex items-center gap-2 text-13 text-muted-foreground">
              <Loader2 className="size-3 shrink-0 animate-spin" aria-hidden="true" />
              {translateUi("Connecting @{{value1}}…", { value1: request?.bot_username })}
            </p>
          )}
          {connecting && request?.connection_error && (
            <p role="status" className="text-13 text-muted-foreground">{request.connection_error}</p>
          )}
        </div>
      ) : <ol aria-label={translateUi("Telegram setup steps")} className="space-y-4">
        <li className="flex gap-3">
          <span
            className="flex size-7 shrink-0 items-center justify-center rounded-full border border-border text-13"
            aria-hidden="true"
          >
            {creating ? <Check className="size-3 text-success" /> : "1"}
          </span>
          <div className="space-y-2">
            <p className="type-row-title">{translateUi("Continue in Telegram")}</p>
            <p className="text-13 text-muted-foreground">{translateUi("Create a new bot for the account selected above. NyxID will connect it automatically when you finish Telegram’s form.")}</p>
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
                  ? translateUi("Reopen Telegram")
                  : translateUi("Continue in Telegram")}
              </Button>
            )}
            {launchUrl && canLaunch && (
              <p className="text-13 text-muted-foreground">{translateUi("Telegram didn’t open?")}{" "}
                <a
                  className="ph-no-capture underline"
                  href={launchUrl}
                  target="_blank"
                  rel="noopener noreferrer"
                  referrerPolicy="no-referrer"
                >{translateUi("Open Telegram")}</a>
              </p>
            )}
          </div>
        </li>
        <li
          className="flex gap-3"
          aria-current={creating && !connected ? "step" : undefined}
        >
          <span
            className="flex size-7 shrink-0 items-center justify-center rounded-full border border-border text-13"
            aria-hidden="true"
          >
            {connected ? <Check className="size-3 text-success" /> : "2"}
          </span>
          <div className="space-y-2">
            <p className="type-row-title">{translateUi("Create your bot")}</p>
            <p className="text-13 text-muted-foreground">{translateUi("In the setup chat, tap ")}<strong>{translateUi("Create bot")}</strong>{translateUi(" and finish Telegram’s name and username form. Tap ")}<strong>{translateUi("Start")}</strong>{" "}{translateUi("first if Telegram asks.")}</p>
            <p className="text-13 text-muted-foreground">{translateUi("You can stay in Telegram. This page updates automatically when the connection is complete.")}</p>
            {connecting && (
              <p role="status" className="flex items-center gap-2 text-14">
                <Loader2 className="size-3 animate-spin" aria-hidden="true" />
                {translateUi("Connecting @{{value1}}…", { value1: request?.bot_username })}
              </p>
            )}
            {connecting && request?.connection_error && (
              <p role="status" className="text-13 text-muted-foreground">
                {request.connection_error}
              </p>
            )}
          </div>
        </li>
      </ol>}
      {request && !request.setup_notice && ["waiting_telegram", "waiting_bot"].includes(request.status) && (
        <TelegramPromptStatus request={request} recoveryUnexpired={recoveryUnexpired} pending={pending || !recoveryReadsReady || !managerUsername} recovering={creationLink.isPending}
          onRecover={(index) => void openCreationForm(index)}
          onRegenerate={() => void regenerate()} regenerating={regenerateUsername.isPending}
          recoveryUrl={recoveryMatchesRequest ? currentRecoveryUrl : undefined} />
      )}
      {request && !terminal && !connected && (
        !fullPage || (!request.setup_notice && request.status === "waiting_bot" && request.can_open_creation_form && recoveryUnexpired)
      ) && (
        <Button type="button" variant="ghost" size="sm" disabled={pending || configuration.isFetching}
          onClick={() => void checkProgress()}>{translateUi("Check progress")}</Button>
      )}
      {request && !request.auto_connect && !terminal && (
        <div className="space-y-2 rounded-lg border border-border p-3 text-13">
          <p>{translateUi("This setup was started with the previous flow. Finish its Telegram approval, then complete the saved connection below")}{fullPage ? "." : translateUi(", or cancel and start again.")}
          </p>
          {connecting && (
            <Button
              type="button"
              variant="primary"
              disabled={pending}
              onClick={() => void finishLegacyRequest()}
            >
              {connect.isPending ? translateUi("Connecting…") : translateUi("Finish saved connection")}
            </Button>
          )}
        </div>
      )}
      {terminal && !request?.setup_notice && (
        <p role="status" className="text-14 text-muted-foreground">
          {request?.status === "expired"
            ? translateUi("This setup expired. Start again to create a new bot.")
            : request?.status === "suspended"
              ? translateUi("This bot’s management changed. Manage the saved bot from Channel Bots.")
              : translateUi("Setup cancelled. You can start again.")}
        </p>
      )}
      {!fullPage && request && !terminal && !connected && (
        <div className="space-y-2 border-t border-border pt-3">
          <p className="text-13 text-muted-foreground">
            {request.setup_notice ? translateUi("Saved setup") : translateUi("Setting up")}: {request.label}.{" "}
            {request.status !== "provisioning" && !request.setup_notice && (
              <>{translateUi("Saved until")}{" "}
                <time dateTime={request.expires_at}>
                  {new Date(request.expires_at).toLocaleTimeString(getFormattingLocale(), {
                    hour: "numeric",
                    minute: "2-digit",
                  })}
                </time>
                .
              </>
            )}
          </p>
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
            >{translateUi("Cancel setup")}</Button>
          )}
        </div>
      )}
    </div>
  );
}
