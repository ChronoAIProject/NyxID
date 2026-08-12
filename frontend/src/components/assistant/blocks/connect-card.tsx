import { useEffect, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { ExternalLink, KeyRound, Loader2, RefreshCw } from "lucide-react";
import { AddKeyDialog } from "@/components/dashboard/add-key-dialog";
import { ManageConnectionModal } from "@/components/assistant/manage-connection-modal";
import { ServiceIcon } from "@/components/service-icon";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  KEY_AUTH_ACTIVE,
  KEY_AUTH_FAILED,
  useCatalogEntry,
  useKeyAuthorizationWatch,
  useKeys,
} from "@/hooks/use-keys";
import { useChatPresence } from "@/hooks/use-chat-presence";
import { ApiError } from "@/lib/api-client";
import { catalogAuthKind } from "@/lib/assistant/plugins";
import { connectWatchDeadline } from "@/lib/assistant/connect-watch";
import type { ConnectCardContentBlock } from "@/types/assistant";

const STATE_LABEL: Record<ConnectCardContentBlock["state"], string> = {
  needs_connection: "Not connected",
  waiting_for_provider: "Authorizing",
  waiting_for_user: "Waiting for you",
  connected: "Connected",
  error: "Failed",
  timed_out: "Timed out",
};

type LocalVerdict = {
  readonly attemptId: string;
  readonly kind: "authorized" | "failed" | "timed_out" | "cancelled";
  readonly message?: string;
  readonly keyId?: string;
  readonly previousAuthorizationAt?: string | null;
};

type PendingAuthorization = {
  readonly keyId: string;
  readonly attemptId: string;
  readonly previousAuthorizationAt: string | null | undefined;
  readonly startedAt: number;
};

/**
 * Compact one-row connection prompt (DESIGN.md banner anatomy: icon tile +
 * text + action, compact density). The icon comes from the same catalog
 * glyph registry the AI Services page uses — `catalog_slug` is the raw
 * NyxID service slug (`api-github`, `llm-openai`) passed straight through.
 *
 * New connections and OAuth reauthorization happen in place through the
 * shared AddKeyDialog. Existing non-OAuth credentials route to their key
 * detail because credential rotation is owned by that management surface.
 */
export function ConnectCard({
  block,
}: {
  readonly block: ConnectCardContentBlock;
}) {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [dialogOpen, setDialogOpen] = useState(false);
  const [managedKeyId, setManagedKeyId] = useState<string | null>(null);
  const [pendingAuth, setPendingAuth] = useState<PendingAuthorization | null>(
    null,
  );
  const pendingAuthRef = useRef<PendingAuthorization | null>(null);
  const [localVerdict, setLocalVerdict] = useState<LocalVerdict | null>(null);
  const settledAttemptRef = useRef<string | null>(null);
  const { visible, lastActivityAt } = useChatPresence();
  const { data: keys } = useKeys();
  // Resolve the real connect modality from the catalog. `block.auth_kind` is
  // display data the live authorization frame can't populate — the transport
  // fills in `"api_key"` as a placeholder — so trusting it offers an OAuth
  // service a paste-your-key button. Card fields are never action inputs.
  const catalogSlug =
    block.catalog_slug !== "custom" ? block.catalog_slug : null;
  const catalogQuery = useCatalogEntry(catalogSlug);
  const catalogEntry = catalogQuery.data;
  const authKind = catalogEntry
    ? catalogAuthKind(catalogEntry)
    : block.auth_kind;
  const serviceName = catalogEntry?.name ?? block.service_name;
  // Only a 404 means "this slug isn't a NyxID service". A 500 or a timeout
  // means we don't know yet — telling the user the service doesn't exist, and
  // removing their way forward, would be a lie the retry can't undo.
  const catalogError = catalogQuery.error;
  const unresolvableSlug =
    catalogSlug !== null &&
    catalogError instanceof ApiError &&
    catalogError.status === 404;
  const catalogUnavailable =
    catalogSlug !== null && catalogQuery.isError && !unresolvableSlug;
  const catalogPending = catalogSlug !== null && !catalogEntry && !catalogError;
  const matchingKey =
    (block.key_id
      ? (keys ?? []).find((key) => key.id === block.key_id)
      : undefined) ??
    (keys ?? []).find((key) => key.catalog_service_slug === block.catalog_slug);
  const needsReauthorization = block.reason_code === "NYXID_UNAUTHORIZED";
  const canManageMatchingKey =
    matchingKey !== undefined &&
    !matchingKey.auto_connected &&
    !(
      matchingKey.credential_source?.type === "org" &&
      matchingKey.credential_source.role !== "admin"
    );
  const reconnectKey =
    needsReauthorization &&
    canManageMatchingKey &&
    (matchingKey.credential_type === "oauth2" ||
      matchingKey.auth_method === "oauth2" ||
      matchingKey.auth_method === "oidc")
      ? matchingKey
      : null;
  const connectedNow =
    !needsReauthorization &&
    block.catalog_slug !== "custom" &&
    (keys ?? []).some(
      (key) =>
        key.status === KEY_AUTH_ACTIVE &&
        key.catalog_service_slug === block.catalog_slug,
    );
  const cancelledAuthorizationAdvanced =
    localVerdict?.kind === "cancelled" &&
    matchingKey !== undefined &&
    matchingKey.id === localVerdict.keyId &&
    matchingKey.status === KEY_AUTH_ACTIVE &&
    (localVerdict.previousAuthorizationAt === undefined ||
      (matchingKey.last_authorized_at != null &&
        matchingKey.last_authorized_at !==
          localVerdict.previousAuthorizationAt));
  const connected =
    connectedNow ||
    block.state === "connected" ||
    localVerdict?.kind === "authorized" ||
    cancelledAuthorizationAdvanced;
  // An authorization is in flight for this service: the placeholder key
  // exists but the provider callback hasn't landed. Surfaced live so the
  // transcript reflects the handoff happening in the other tab.
  const authorizing =
    !connected &&
    localVerdict === null &&
    (pendingAuth !== null || matchingKey?.status === "pending_auth");
  const failed =
    !connected &&
    (localVerdict?.kind === "failed" ||
      (localVerdict === null && block.state === "error"));
  const timedOut =
    !connected &&
    (localVerdict?.kind === "timed_out" ||
      (localVerdict === null && block.state === "timed_out"));
  const guidance = connected
    ? "Connected — send your request again."
    : authorizing
      ? `Waiting for ${serviceName}…`
      : localVerdict?.kind === "failed"
        ? (localVerdict.message ??
          "Authorization failed. Try the connection again.")
        : localVerdict?.kind === "timed_out"
          ? "Authorization timed out. Start the connection again when ready."
          : localVerdict?.kind === "cancelled"
            ? "Connection cancelled — you can start again."
            : unresolvableSlug
              ? "This service isn't in your NyxID catalog — add it in AI Services."
              : catalogUnavailable
                ? "Couldn't reach the NyxID catalog — retry in a moment."
                : (block.error_message ??
                  block.steps[0]?.body ??
                  block.subtitle);
  const actionLabel = reconnectKey
    ? "Reconnect"
    : needsReauthorization && matchingKey
      ? "Manage"
      : "Connect";
  const stateLabel =
    localVerdict?.kind === "failed"
      ? STATE_LABEL.error
      : localVerdict?.kind === "timed_out"
        ? STATE_LABEL.timed_out
        : authorizing
          ? STATE_LABEL.waiting_for_provider
          : needsReauthorization
            ? "Reauthorization required"
            : STATE_LABEL[block.state];
  // Catalog state gates only a *fresh* connect, where picking the wrong
  // modality from the block's hint is the risk. Reconnect and Manage key off
  // an existing NyxID row and need no catalog — hiding them on a 404 would
  // strip the only recovery path from a key the user still owns.
  const awaitingCatalog = catalogPending && matchingKey === undefined;
  const blockedByCatalog = unresolvableSlug && matchingKey === undefined;

  const watch = useKeyAuthorizationWatch(pendingAuth?.keyId ?? null, {
    attemptId: pendingAuth?.attemptId ?? "idle",
    previousAuthorizationAt: pendingAuth?.previousAuthorizationAt,
    enabled: pendingAuth !== null && visible,
    deadlineAt: pendingAuth
      ? connectWatchDeadline(pendingAuth.startedAt, lastActivityAt)
      : 0,
  });

  useEffect(() => {
    const attempt = pendingAuth;
    if (!attempt || settledAttemptRef.current === attempt.attemptId) return;

    let verdict: LocalVerdict | null = null;
    if (watch.authorized) {
      verdict = { attemptId: attempt.attemptId, kind: "authorized" };
    } else if (watch.status === KEY_AUTH_FAILED) {
      verdict = {
        attemptId: attempt.attemptId,
        kind: "failed",
        ...(watch.errorMessage ? { message: watch.errorMessage } : {}),
      };
    } else if (watch.timedOut) {
      verdict = { attemptId: attempt.attemptId, kind: "timed_out" };
    }
    if (!verdict) return;

    settledAttemptRef.current = attempt.attemptId;
    void Promise.resolve().then(() => {
      if (settledAttemptRef.current !== attempt.attemptId) return;
      setLocalVerdict(verdict);
      if (pendingAuthRef.current?.attemptId === attempt.attemptId) {
        pendingAuthRef.current = null;
      }
      setPendingAuth((current) =>
        current?.attemptId === attempt.attemptId ? null : current,
      );
    });
  }, [
    pendingAuth,
    watch.authorized,
    watch.errorMessage,
    watch.status,
    watch.timedOut,
  ]);

  // Contract fields the compact row has no place for. Rendered plainly —
  // deliberately unstyled for now — so a block carrying them is not silently
  // dropped on the floor. The visual pass comes later; what matters today is
  // that the FE honours every field §3.5 defines.
  const extraSteps = block.steps.length > 1 ? block.steps : [];
  const showDeviceCode =
    !connected &&
    Boolean(block.device_user_code ?? block.device_verification_url);
  const showScopes =
    block.requested_scopes.length > 0 ||
    (block.granted_scopes?.length ?? 0) > 0;
  const hasDetail =
    showDeviceCode ||
    extraSteps.length > 0 ||
    showScopes ||
    Boolean(block.footer);

  return (
    <section className="rounded-xl border border-border/70 bg-card">
      <div className="flex items-center gap-3 px-4 py-3">
        <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-hairline bg-overlay-strong">
          <ServiceIcon slug={block.catalog_slug} size="sm" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <p className="truncate text-[13px] font-semibold text-foreground">
              {serviceName}
            </p>
            <Badge
              variant={
                connected ? "success" : failed ? "destructive" : "warning"
              }
            >
              {connected
                ? "Connected"
                : timedOut
                  ? STATE_LABEL.timed_out
                  : stateLabel}
            </Badge>
          </div>
          <p
            className="flex items-center gap-1.5 truncate text-[11px] text-muted-foreground"
            role={authorizing ? "status" : undefined}
            aria-live={authorizing ? "polite" : undefined}
          >
            {authorizing && (
              <Loader2 className="h-3 w-3 shrink-0 animate-spin text-text-tertiary" />
            )}
            {guidance}
          </p>
        </div>
        {/* Hidden while an authorization is already in flight: a second click
          would mint a second placeholder key for the same service. */}
        {!connected &&
          !blockedByCatalog &&
          !authorizing &&
          !awaitingCatalog && (
            <Button
              variant="primary"
              size="sm"
              className="shrink-0"
              onClick={() => {
                if (needsReauthorization && matchingKey && !reconnectKey) {
                  void navigate({
                    to: "/keys/$keyId",
                    params: { keyId: matchingKey.id },
                  });
                  return;
                }
                setDialogOpen(true);
              }}
            >
              {reconnectKey ? (
                <RefreshCw />
              ) : authKind === "api_key" ? (
                <KeyRound />
              ) : (
                <ExternalLink />
              )}
              {actionLabel}
            </Button>
          )}
      </div>

      {hasDetail && (
        <div className="space-y-2 border-t border-border/50 px-4 py-2.5 text-[11px] text-muted-foreground">
          {showDeviceCode && (
            <div className="space-y-1">
              {block.device_user_code && (
                <p>
                  Enter code{" "}
                  <code className="rounded bg-overlay px-1 py-0.5 font-mono text-foreground">
                    {block.device_user_code}
                  </code>
                </p>
              )}
              {block.device_verification_url && (
                <a
                  href={block.device_verification_url}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="inline-flex items-center gap-1 text-foreground underline underline-offset-2"
                >
                  {block.device_verification_url}
                  <ExternalLink className="h-3 w-3" />
                </a>
              )}
            </div>
          )}

          {extraSteps.length > 0 && (
            <ol className="space-y-0.5">
              {extraSteps.map((step, index) => (
                <li key={`${step.title}-${String(index)}`}>
                  {step.done ? "✓" : `${String(index + 1)}.`} {step.title}
                  {step.body ? ` — ${step.body}` : ""}
                </li>
              ))}
            </ol>
          )}

          {showScopes && (
            <p>
              {(block.granted_scopes?.length ?? 0) > 0
                ? `Granted: ${(block.granted_scopes ?? []).join(", ")}`
                : `Requests: ${block.requested_scopes.join(", ")}`}
            </p>
          )}

          {block.footer && <p className="text-text-tertiary">{block.footer}</p>}
        </div>
      )}

      <AddKeyDialog
        open={dialogOpen}
        onOpenChange={setDialogOpen}
        launch="popup"
        flow="cc"
        onPopupViewResult={(keyId) => {
          setDialogOpen(false);
          window.setTimeout(() => setManagedKeyId(keyId), 0);
          return true;
        }}
        prefillSlug={
          !reconnectKey && block.catalog_slug !== "custom"
            ? block.catalog_slug
            : undefined
        }
        reconnectKey={reconnectKey}
        onAuthorizationPending={(attempt) => {
          const pending = { ...attempt, startedAt: Date.now() };
          settledAttemptRef.current = null;
          pendingAuthRef.current = pending;
          setPendingAuth(pending);
          setLocalVerdict(null);
        }}
        onAuthorizationAborted={(attemptId) => {
          const current = pendingAuthRef.current;
          if (
            current?.attemptId !== attemptId ||
            settledAttemptRef.current === attemptId
          ) {
            return;
          }
          settledAttemptRef.current = attemptId;
          pendingAuthRef.current = null;
          setPendingAuth(null);
          setLocalVerdict({
            attemptId,
            kind: "cancelled",
            keyId: current.keyId,
            previousAuthorizationAt: current.previousAuthorizationAt,
          });
          void queryClient.invalidateQueries({
            queryKey: ["keys"],
            exact: true,
          });
          void queryClient.invalidateQueries({
            queryKey: ["keys", current.keyId],
            exact: true,
          });
        }}
      />
      {managedKeyId && (
        <ManageConnectionModal
          keyIds={[managedKeyId]}
          serviceName={serviceName}
          iconSlug={block.catalog_slug}
          onClose={() => setManagedKeyId(null)}
        />
      )}
    </section>
  );
}
