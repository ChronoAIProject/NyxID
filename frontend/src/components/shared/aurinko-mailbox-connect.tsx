import { useEffect, useId, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { ErrorBanner } from "./error-banner";
import {
  authorizeAurinkoMailbox,
  cancelAurinkoAuthorization,
  listAurinkoMailboxes,
  useAurinkoMailboxes,
} from "@/hooks/use-aurinko-mailboxes";
import {
  openOAuthChannel,
  openOAuthPopup,
  postOAuthAck,
} from "@/lib/oauth-popup";
import {
  isOAuthResultMessage,
  validateAuthorizationUrl,
} from "@/schemas/oauth-popup";
import {
  AURINKO_PROVIDERS,
  aurinkoProviderSchema,
  type AurinkoProvider,
  type AurinkoConnection,
  type AurinkoAuthorization,
} from "@/schemas/aurinko-mailboxes";

export function AurinkoProviderSelect({
  value,
  onChange,
  disabled = false,
}: {
  readonly value: AurinkoProvider;
  readonly onChange: (value: AurinkoProvider) => void;
  readonly disabled?: boolean;
}) {
  const id = useId();
  return (
    <div className="space-y-2">
      <Label htmlFor={id}>Email provider</Label>
      <Select
        value={value}
        onValueChange={(next) => onChange(aurinkoProviderSchema.parse(next))}
        disabled={disabled}
      >
        <SelectTrigger id={id}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {Object.entries(AURINKO_PROVIDERS).map(([key, label]) => (
            <SelectItem key={key} value={key}>
              {label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      <p className="text-xs text-muted-foreground">
        {value === "IMAP"
          ? "Enter IMAP and SMTP settings only on Aurinko’s secure hosted form. IMAP must be enabled; POP3-only accounts are unsupported."
          : value === "EWS" || value === "iCloud"
            ? "Enter the credentials requested for your mailbox only on Aurinko’s secure hosted form."
            : "Sign in to your email provider through Aurinko. Your platform administrator configures the application credentials."}
      </p>
    </div>
  );
}

interface PreparedConnection extends AurinkoConnection {
  readonly previousAuthorizationAt?: string | null;
  readonly discardPending?: () => Promise<void>;
}

export function AurinkoMailboxConnect({
  label,
  ownerId,
  connectionId,
  allowReuse = false,
  disabled = false,
  prepare,
  onStarted,
  onAborted,
  onConnected,
}: {
  readonly label: string;
  readonly ownerId?: string | null;
  readonly connectionId?: string | null;
  readonly allowReuse?: boolean;
  readonly disabled?: boolean;
  readonly prepare?: () => Promise<PreparedConnection>;
  readonly onStarted?: (
    started: AurinkoAuthorization,
    previousAuthorizationAt: string | null | undefined,
  ) => void;
  readonly onAborted?: (nonce: string) => void;
  readonly onConnected: (
    connection: AurinkoConnection,
    signal: AbortSignal,
  ) => void | Promise<void>;
}) {
  const [provider, setProvider] = useState<AurinkoProvider>("Google");
  const [selectedId, setSelectedId] = useState("new");
  const [stage, setStage] = useState<
    "starting" | "authorizing" | "connecting" | "cancelling" | null
  >(null);
  const [error, setError] = useState<string | null>(null);
  const query = useAurinkoMailboxes(
    ownerId,
    allowReuse || Boolean(connectionId),
  );
  const client = useQueryClient();
  const operation = useRef<{ cancel: () => Promise<void> } | null>(null);
  const mounted = useRef(false);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const selectorId = useId();
  const aborted = useRef(onAborted);
  useEffect(() => {
    aborted.current = onAborted;
  }, [onAborted]);
  useEffect(
    () => () => {
      const current = operation.current;
      if (!current) return;
      void current.cancel().then(() => {
        if (mounted.current && operation.current === null) setStage(null);
      }).catch(() => {
        if (!mounted.current || operation.current !== current) return;
        setStage("cancelling");
        setError("Cancellation could not be confirmed. Retry cancellation before starting another sign-in. Unused attempts expire after ten minutes.");
      });
    },
    [ownerId, connectionId],
  );
  const existing = query.data?.find(
    (mailbox) => mailbox.connection_id === connectionId,
  );
  const reusable =
    query.data?.filter(
      (mailbox) => mailbox.is_active && mailbox.status === "active",
    ) ?? [];
  const providerValue = existing?.service_type ?? provider;
  const locked = Boolean(existing?.service_type);
  const missingConnection = Boolean(
    connectionId && !query.isPending && !query.isError && !existing,
  );

  async function cancel() {
    const current = operation.current;
    if (!current) return;
    setStage("cancelling");
    try {
      await current.cancel();
      if (mounted.current) setStage(null);
    } catch {
      if (!mounted.current) return;
      setError(
        "Cancellation could not be confirmed. Retry cancellation before starting another sign-in. Unused attempts expire after ten minutes.",
      );
    }
  }

  function connect() {
    if (operation.current || disabled) return;
    const reuse = allowReuse && !connectionId && selectedId !== "new";
    const popup = reuse ? null : openOAuthPopup();
    if (!reuse && !popup) {
      setError("Allow popups for this site, then try again.");
      return;
    }
    setError(null);
    setStage(reuse ? "connecting" : "starting");
    const controller = new AbortController();
    let cancelled = false;
    let finished = false;
    let authorizationFinished = false;
    let completing = false;
    let channel: BroadcastChannel | null = null;
    let attempt: AurinkoAuthorization | null = null;
    let prepared: PreparedConnection | undefined;
    let cancellation: Promise<void> | null = null;
    const close = () => {
      controller.abort();
      channel?.close();
      popup?.close();
      window.clearTimeout(timer);
    };
    const current = {
      cancel: () => {
        if (finished) return Promise.resolve();
        if (cancellation) return cancellation;
        cancelled = true;
        close();
        cancellation = (async () => {
          // Await a delayed start response so its server nonce can also be invalidated.
          await starting.catch(() => undefined);
          if (attempt && !authorizationFinished)
            await cancelAurinkoAuthorization(attempt.attempt_nonce);
          if (!authorizationFinished) await prepared?.discardPending?.();
          if (attempt && !authorizationFinished)
            aborted.current?.(attempt.attempt_nonce);
          finished = true;
          if (operation.current === current) operation.current = null;
        })().catch((cause: unknown) => {
          cancellation = null;
          throw cause;
        });
        return cancellation;
      },
    };
    operation.current = current;
    const active = () =>
      !cancelled && !finished && operation.current === current;
    const fail = (cause: unknown) => {
      if (!active()) return;
      const message =
        cause instanceof Error
          ? cause.message
          : "Unable to connect this mailbox";
      setError(message);
      setStage("cancelling");
      void current
        .cancel()
        .then(() => { if (mounted.current) setStage(null); })
        .catch(() => {
          if (!mounted.current) return;
          setError(
            `${message} Cancellation could not be confirmed; retry cancellation.`,
          );
        });
    };
    const timer = window.setTimeout(
      () => fail(new Error("Mailbox sign-in timed out. Connect again.")),
      10 * 60_000,
    );
    const complete = async (connection: AurinkoConnection) => {
      if (!active()) return;
      authorizationFinished = true;
      if (allowReuse && !connectionId) setSelectedId(connection.connection_id);
      if (channel) postOAuthAck(channel);
      await Promise.all([
        client.invalidateQueries({ queryKey: ["keys"] }),
        client.invalidateQueries({ queryKey: ["aurinko-mailboxes"] }),
      ]);
      if (!active()) return;
      await onConnected(connection, controller.signal);
      if (!active()) return;
      finished = true;
      close();
      operation.current = null;
      setStage(null);
    };
    const starting = (async () => {
      if (reuse) {
        const mailboxes = await listAurinkoMailboxes(ownerId);
        const mailbox = mailboxes.find(
          (item) =>
            item.connection_id === selectedId &&
            item.is_active &&
            item.status === "active",
        );
        if (!mailbox)
          throw new Error(
            "This mailbox is no longer available. Refresh the connection list.",
          );
        await complete(mailbox);
        return;
      }
      prepared = await prepare?.();
      if (!active()) return;
      const started = await authorizeAurinkoMailbox({
        provider: providerValue,
        label: label.trim(),
        ...(ownerId ? { owner_id: ownerId } : {}),
        ...(prepared?.connection_id || connectionId
          ? { connection_id: prepared?.connection_id ?? connectionId! }
          : {}),
      });
      attempt = started;
      if (!active()) return;
      const authorizationUrl = validateAuthorizationUrl(
        started.authorization_url,
        started.attempt_nonce,
        "https://api.aurinko.io",
      );
      if (
        !authorizationUrl ||
        authorizationUrl.pathname !== "/v1/auth/authorize"
      ) {
        throw new Error("NyxID returned an invalid mailbox authorization URL");
      }
      channel = openOAuthChannel(started.attempt_nonce);
      if (!channel)
        throw new Error(
          "This browser cannot complete mailbox sign-in. Use a browser with BroadcastChannel support.",
        );
      onStarted?.(started, prepared?.previousAuthorizationAt);
      channel.onmessage = (event: MessageEvent<unknown>) => {
        if (
          !active() ||
          completing ||
          !isOAuthResultMessage(event.data) ||
          event.data.flow !== "cc"
        )
          return;
        if (event.data.status === "error") {
          fail(
            new Error(
              "Mailbox authorization failed or was declined. Try again.",
            ),
          );
          return;
        }
        completing = true;
        setStage("connecting");
        void (async () => {
          const mailboxes = await listAurinkoMailboxes(ownerId);
          const mailbox = mailboxes.find(
            (item) =>
              item.connection_id === started.connection_id &&
              item.service_id === started.service_id &&
              item.is_active &&
              item.status === "active",
          );
          if (!mailbox)
            throw new Error(
              "Mailbox authorization has not completed. Connect again.",
            );
          await complete(mailbox);
        })().catch(fail);
      };
      setStage("authorizing");
      await popup!.navigate(
        started.authorization_url,
        started.attempt_nonce,
        "Aurinko Email",
        true,
      );
    })();
    void starting.catch(fail);
  }

  return (
    <div className="space-y-4">
      {error && <ErrorBanner message={error} />}
      {query.isError && (
        <ErrorBanner
          message="Unable to load mailbox connections"
          onRetry={() => void query.refetch()}
        />
      )}
      {missingConnection && (
        <ErrorBanner message="The mailbox connection no longer exists or is not available to this owner." />
      )}
      {allowReuse && !connectionId && (
        <div className="space-y-2">
          <Label htmlFor={selectorId}>Mailbox connection</Label>
          <Select
            value={selectedId}
            onValueChange={setSelectedId}
            disabled={Boolean(stage) || query.isPending || query.isError}
          >
            <SelectTrigger id={selectorId}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="new">Connect a new mailbox</SelectItem>
              {reusable.map((mailbox) => (
                <SelectItem
                  key={mailbox.connection_id}
                  value={mailbox.connection_id}
                >
                  {mailbox.mailbox_address ?? mailbox.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      )}
      {(connectionId || selectedId === "new") && (
        <AurinkoProviderSelect
          value={providerValue}
          onChange={setProvider}
          disabled={Boolean(stage) || locked || disabled}
        />
      )}
      {existing?.mailbox_address && (
        <p className="text-sm">Reconnect {existing.mailbox_address}</p>
      )}
      {existing && !existing.is_active && (
        <p className="text-sm text-muted-foreground">
          Enable this AI Service before reconnecting its mailbox.
        </p>
      )}
      <div className="flex gap-2">
        <Button
          type="button"
          variant="primary"
          onClick={connect}
          isLoading={Boolean(stage)}
          disabled={
            disabled ||
            !label.trim() ||
            label.trim().length > 128 ||
            missingConnection ||
            Boolean(existing && !existing.is_active) ||
            ((Boolean(connectionId) || allowReuse) &&
              (query.isPending || query.isError))
          }
        >
          {stage === "cancelling"
            ? "Cancelling..."
            : stage === "starting"
              ? "Opening sign-in..."
              : stage === "authorizing"
                ? "Waiting for authorization..."
                : stage === "connecting"
                  ? "Connecting mailbox..."
                  : connectionId
                    ? "Reconnect mailbox"
                    : selectedId !== "new"
                      ? "Use selected mailbox"
                      : "Connect mailbox"}
        </Button>
        {stage && (
          <Button type="button" variant="ghost" onClick={() => void cancel()}>
            {stage === "cancelling" ? "Retry cancellation" : "Cancel"}
          </Button>
        )}
      </div>
    </div>
  );
}
