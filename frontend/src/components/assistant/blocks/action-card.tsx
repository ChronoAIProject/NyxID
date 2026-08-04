import { useEffect, useRef, useState } from "react";
import {
  AlertTriangle,
  Globe,
  Loader2,
  Server,
  ShieldCheck,
  X,
} from "lucide-react";
import { AddKeyDialog } from "@/components/dashboard/add-key-dialog";
import { ServiceIcon } from "@/components/service-icon";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useChatPresence } from "@/hooks/use-chat-presence";
import {
  KEY_AUTH_ACTIVE,
  KEY_AUTH_FAILED,
  useKeyAuthorizationWatch,
} from "@/hooks/use-keys";
import {
  actionServiceLabel,
  clampServiceLabel,
  descriptorForAction,
} from "@/lib/assistant/action-registry";
import { connectWatchDeadline } from "@/lib/assistant/connect-watch";
import type { ActionReport } from "@/schemas/assistant-actions";
import type { ActionCardContentBlock } from "@/types/assistant";

interface ActionCardProps {
  readonly block: ActionCardContentBlock;
  readonly onProgress: (blockId: string, inProgress: boolean) => void;
  readonly onBlock: (blockId: string, note: string) => Promise<void> | void;
  readonly onResolve: (report: ActionReport) => Promise<void> | void;
}

const VERIFICATION_BLOCKED_NOTE =
  "Connected, but NyxID could not verify which service was created. Manage it in AI Services, then ask the assistant to request it again.";

const AUTHORIZATION_TIMEOUT_NOTE =
  "NyxID stopped waiting for this connection to finish authorizing. If you did complete it, find it in AI Services — otherwise ask the assistant to request it again.";

function authorizationFailedNote(reason: string | undefined): string {
  const detail = reason?.trim();
  return detail
    ? `Authorization did not complete: ${detail}`
    : "Authorization did not complete. Ask the assistant to request this service again.";
}

function ParameterSummary({
  block,
}: {
  readonly block: ActionCardContentBlock;
}) {
  const params = block.params;
  if (params.variant === "unknown") return null;
  const scopes =
    params.variant === "catalog" ? params.requested_scopes.filter(Boolean) : [];
  let endpointHost = "";
  if (params.variant === "custom" && params.endpoint_url) {
    try {
      endpointHost = new URL(params.endpoint_url).host;
    } catch {
      endpointHost = "";
    }
  }

  return (
    <div className="space-y-2.5 border-y border-border bg-muted px-4 py-3">
      <div className="flex flex-wrap items-center gap-1.5">
        <span className="text-[10px] font-semibold uppercase tracking-[1px] text-muted-foreground">
          Service
        </span>
        <Badge variant="secondary" className="max-w-full truncate">
          <span className="min-w-0 truncate">
            {clampServiceLabel(
              params.variant === "catalog" ? params.service_slug : params.name,
            ) || "Custom"}
          </span>
        </Badge>
        {endpointHost ? (
          <Badge variant="secondary" className="max-w-full truncate">
            <Globe className="mr-1 h-3 w-3" />
            <span className="min-w-0 truncate">{endpointHost}</span>
          </Badge>
        ) : null}
      </div>
      {scopes.length > 0 ? (
        <div className="flex flex-wrap items-center gap-1.5">
          <span className="text-[10px] font-semibold uppercase tracking-[1px] text-muted-foreground">
            Scopes
          </span>
          {scopes.map((scope) => (
            <Badge
              key={scope}
              variant="secondary"
              className="max-w-full truncate font-mono"
            >
              <span className="min-w-0 truncate">{scope}</span>
            </Badge>
          ))}
        </div>
      ) : null}
      {params.via_node_id || params.target_org_id ? (
        <div className="flex flex-wrap items-center gap-1.5">
          {/* Ids are wire-supplied and only length-capped at 256 chars, so they
              stay inside the card instead of widening the chat column. */}
          {params.via_node_id ? (
            <Badge variant="secondary" className="max-w-full truncate font-mono">
              <Server className="mr-1 h-3 w-3" />
              <span className="min-w-0 truncate">
                Node {params.via_node_id}
              </span>
            </Badge>
          ) : null}
          {params.target_org_id ? (
            <Badge variant="secondary" className="max-w-full truncate font-mono">
              <span className="min-w-0 truncate">
                Org {params.target_org_id}
              </span>
            </Badge>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

const RECEIPT = {
  completed: {
    title: "Reported — awaiting assistant verification",
    icon: ShieldCheck,
    container: "border-border bg-overlay",
    iconClass: "text-nyx-secondary-400",
  },
  declined: {
    title: "Action declined",
    icon: X,
    container: "border-border bg-overlay",
    iconClass: "text-muted-foreground",
  },
  failed: {
    title: "Connection failed",
    icon: AlertTriangle,
    container: "border-destructive/30 bg-destructive/[0.06]",
    iconClass: "text-destructive",
  },
} as const;

function Receipt({ block }: { readonly block: ActionCardContentBlock }) {
  if (
    block.status !== "completed" &&
    block.status !== "declined" &&
    block.status !== "failed"
  ) {
    return null;
  }
  const receipt = RECEIPT[block.status];
  const Icon = receipt.icon;
  return (
    <section className={`rounded-xl border p-4 ${receipt.container}`}>
      <div className="flex items-center gap-2 text-[12px] font-semibold text-foreground">
        <Icon className={`h-4 w-4 ${receipt.iconClass}`} />
        {receipt.title}
      </div>
      <p className="mt-2 text-[12px] leading-relaxed text-muted-foreground">
        {block.outcome_note}
      </p>
      <p className="mt-1.5 text-[10px] text-text-tertiary">
        {actionServiceLabel(block.params)}
      </p>
    </section>
  );
}

function StatusNotice({ block }: { readonly block: ActionCardContentBlock }) {
  if (block.status !== "blocked" && block.status !== "conflicted") {
    return null;
  }
  return (
    <div className="flex items-start gap-2 border-t border-border bg-muted px-4 py-3">
      <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0 text-warning" />
      <p className="text-[11px] leading-relaxed text-muted-foreground">
        {block.outcome_note}
      </p>
    </div>
  );
}

export function ActionCard({
  block,
  onProgress,
  onBlock,
  onResolve,
}: ActionCardProps) {
  const [dialogOpen, setDialogOpen] = useState(false);
  const resolvingRef = useRef(false);
  /**
   * An out-of-band authorization handed to a provider, still settling. Held on
   * the card rather than in the dialog because the dialog is the short-lived
   * surface: the user goes to GitHub, comes back to the chat, and never
   * reopens it. `startedAt` anchors both the poll cadence and the deadline.
   */
  const [pendingAuth, setPendingAuth] = useState<{
    readonly keyId: string;
    readonly startedAt: number;
  } | null>(null);
  /** Guards one auto-settlement per watched key against effect re-entry. */
  const watchSettledRef = useRef<string | null>(null);
  const { visible, lastActivityAt } = useChatPresence();

  useEffect(() => {
    if (block.status === "pending") {
      resolvingRef.current = false;
    }
  }, [block.status]);

  const settled =
    block.status === "completed" ||
    block.status === "declined" ||
    block.status === "failed";

  const watch = useKeyAuthorizationWatch(pendingAuth?.keyId ?? null, {
    // Presence gate: a hidden tab stops polling and resumes on focus.
    enabled: pendingAuth !== null && !settled && visible,
    deadlineAt: pendingAuth
      ? connectWatchDeadline(pendingAuth.startedAt, lastActivityAt)
      : 0,
  });

  // The card's own `wait_for_authorized_key`. `active` is the same verdict the
  // user would have produced by clicking through Continue and Done, so take it
  // directly instead of requiring those clicks; `failed` and the deadline both
  // surface on the card rather than leaving it waiting in silence.
  useEffect(() => {
    const keyId = pendingAuth?.keyId;
    if (!keyId || settled || watchSettledRef.current === keyId) return;

    // The ref is the synchronous re-entry guard; the state clear rides the
    // microtask so this effect never sets the state it depends on inline.
    if (watch.status === KEY_AUTH_ACTIVE) {
      watchSettledRef.current = keyId;
      resolvingRef.current = true;
      void Promise.resolve()
        .then(() => {
          setPendingAuth(null);
          return onResolve({
            actionRequestId: block.action_request_id,
            originTurnId: block.origin_turn_id,
            disposition: "completed",
            resource: { userService: { userServiceId: keyId } },
          });
        })
        .catch(() => {
          resolvingRef.current = false;
          onProgress(block.block_id, false);
        });
      return;
    }

    if (watch.status === KEY_AUTH_FAILED || watch.timedOut) {
      watchSettledRef.current = keyId;
      const note =
        watch.status === KEY_AUTH_FAILED
          ? authorizationFailedNote(watch.errorMessage)
          : AUTHORIZATION_TIMEOUT_NOTE;
      void Promise.resolve()
        .then(() => {
          setPendingAuth(null);
          return onBlock(block.block_id, note);
        })
        .catch(() => undefined);
    }
  }, [
    pendingAuth,
    settled,
    watch.status,
    watch.timedOut,
    watch.errorMessage,
    block.block_id,
    block.action_request_id,
    block.origin_turn_id,
    onResolve,
    onBlock,
    onProgress,
  ]);

  const descriptor = descriptorForAction(
    block.action,
    block.params,
    block.status !== "unsupported",
  );

  if (settled) {
    return <Receipt block={block} />;
  }

  // Trust the descriptor too: a card whose verb has no journey behind it must
  // never render a CTA, whatever status the block carries.
  const unsupported =
    block.status === "unsupported" || descriptor.risk === "unsupported";
  const busy = block.status === "in_progress";
  /** Dialog dismissed, provider not finished: the watch is carrying this. */
  const awaitingAuthorization = pendingAuth !== null && !dialogOpen;
  const blocked = block.status === "blocked";
  const conflicted = block.status === "conflicted";
  const primaryDisabled = busy || blocked || conflicted;
  const secondaryDisabled = busy || conflicted;
  const params = block.params;

  function setOpen(next: boolean) {
    setDialogOpen(next);
    // Closing the dialog on a still-settling authorization is NOT abandonment:
    // the provider tab is open, the watch is running, and the card resolves
    // itself. Rolling back to `pending` here is what used to lose a connection
    // the user had actually completed — and invite a duplicate on the retry.
    if (
      !next &&
      !resolvingRef.current &&
      !pendingAuth &&
      block.status === "in_progress"
    ) {
      onProgress(block.block_id, false);
    }
  }

  function report(
    disposition: "completed" | "declined" | "failed",
    userServiceId?: string,
  ) {
    // A manual outcome supersedes any watch still running for this card.
    if (pendingAuth) {
      watchSettledRef.current = pendingAuth.keyId;
      setPendingAuth(null);
    }
    if (disposition === "completed" && !userServiceId?.trim()) {
      resolvingRef.current = true;
      void Promise.resolve()
        .then(() => onBlock(block.block_id, VERIFICATION_BLOCKED_NOTE))
        .catch(() => undefined)
        .finally(() => {
          resolvingRef.current = false;
        });
      return;
    }
    resolvingRef.current = true;
    const base = {
      actionRequestId: block.action_request_id,
      originTurnId: block.origin_turn_id,
      disposition,
    } as const;
    void Promise.resolve()
      .then(() =>
        onResolve(
          disposition === "completed" && userServiceId
            ? {
                ...base,
                resource: { userService: { userServiceId } },
              }
            : base,
        ),
      )
      .catch(() => {
        // The transport retains failed/rejected reports for retry, and the
        // page has already toasted the delivery failure. Unlock dismissal AND
        // roll the card out of any busy projection: a completed-connection
        // report that dies must not strand the card at "Connecting" with its
        // controls disabled — back to actionable is what makes retry possible.
        resolvingRef.current = false;
        onProgress(block.block_id, false);
      });
  }

  return (
    <section className="overflow-hidden rounded-xl border border-border bg-card shadow-sm">
      <div className="flex items-start gap-3 px-4 py-3.5">
        <div
          className={`flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border ${
            unsupported
              ? "border-destructive/30 bg-destructive/10"
              : "border-nyx-secondary-400/30 bg-nyx-secondary-400/10"
          }`}
        >
          {params.variant === "catalog" ? (
            <ServiceIcon slug={params.service_slug} size="sm" />
          ) : params.variant === "custom" ? (
            <Globe className="h-4 w-4 text-muted-foreground" />
          ) : (
            <AlertTriangle className="h-4 w-4 text-destructive" />
          )}
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="text-[13px] font-semibold text-foreground">
              {descriptor.title(params)}
            </h3>
            <Badge
              variant={
                unsupported || conflicted
                  ? "destructive"
                  : blocked
                    ? "warning"
                    : "accent"
              }
            >
              {unsupported
                ? "Unsupported"
                : conflicted
                  ? "Conflict"
                  : blocked
                    ? "Blocked"
                    : awaitingAuthorization
                      ? "Authorizing"
                      : busy
                        ? "In progress"
                        : "Action required"}
            </Badge>
          </div>
          <p className="mt-1.5 text-[12px] leading-relaxed text-muted-foreground">
            {descriptor.body(params)}
          </p>
        </div>
      </div>

      <ParameterSummary block={block} />
      <StatusNotice block={block} />

      {!unsupported ? (
        <div className="flex items-start gap-2 px-4 py-3">
          <ShieldCheck className="mt-0.5 h-3.5 w-3.5 shrink-0 text-nyx-secondary-400" />
          <p className="text-[11px] leading-relaxed text-muted-foreground">
            You choose the account, routing, and credential. The assistant
            receives only brokered access after you finish.
          </p>
        </div>
      ) : null}

      <div className="flex flex-wrap items-center gap-2 border-t border-border bg-muted px-4 py-3">
        {!unsupported ? (
          <Button
            type="button"
            variant="primary"
            size="sm"
            disabled={primaryDisabled}
            onClick={() => {
              onProgress(block.block_id, true);
              setDialogOpen(true);
            }}
          >
            {busy ? <Loader2 className="animate-spin" /> : <ShieldCheck />}
            {awaitingAuthorization
              ? "Waiting for authorization"
              : busy
                ? "Connecting"
                : descriptor.cta(params)}
          </Button>
        ) : null}
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={secondaryDisabled}
          onClick={() => report("declined")}
        >
          <X />
          Decline
        </Button>
        {blocked ? (
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={secondaryDisabled}
            onClick={() => report("failed")}
          >
            <AlertTriangle />
            Report failure
          </Button>
        ) : null}
        <span className="ml-auto text-[10px] text-muted-foreground">
          Nothing is shared until you finish.
        </span>
      </div>

      {!unsupported ? (
        <AddKeyDialog
          open={dialogOpen}
          onOpenChange={setOpen}
          prefillSlug={
            params.variant === "catalog" ? params.service_slug : undefined
          }
          prefillIncludeAllCatalog={params.variant === "catalog"}
          prefillNodeId={
            params.variant === "unknown"
              ? undefined
              : (params.via_node_id ?? undefined)
          }
          prefillTargetOrgId={
            params.variant === "unknown"
              ? undefined
              : (params.target_org_id ?? undefined)
          }
          prefillCustom={
            params.variant === "custom"
              ? {
                  name: params.name,
                  endpointUrl: params.endpoint_url,
                  authMethod: params.auth_method,
                  authKeyName: params.auth_key_name,
                }
              : undefined
          }
          onSuccess={({ userServiceId }) => report("completed", userServiceId)}
          onAuthorizationPending={(keyId) => {
            watchSettledRef.current = null;
            setPendingAuth({ keyId, startedAt: Date.now() });
          }}
        />
      ) : null}
    </section>
  );
}
