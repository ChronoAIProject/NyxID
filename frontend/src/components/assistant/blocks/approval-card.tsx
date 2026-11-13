import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { ExecutionPermissionCard } from "./execution-permission-card";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { STATUS_CARD_MAX_W } from "@/lib/assistant/banner";
import {
  Ban,
  Check,
  Clock3,
  KeyRound,
  Loader2,
  RotateCcw,
  ShieldAlert,
  X,
} from "lucide-react";
import { ServiceIcon } from "@/components/service-icon";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import type {
  ApprovalCardContentBlock,
  ApprovalDecision,
} from "@/types/assistant";

const CLICK_THROTTLE_MS = 750;
const URGENT_THRESHOLD_MS = 5 * 60_000;

function remainingLabel(remainingMs: number): string {
  // NaN when the server didn't send a parseable expiry (e.g. an older
  // backend without expires_at on the listing) — omit the countdown.
  if (Number.isNaN(remainingMs)) return "";
  if (remainingMs <= 0) return "expired";
  return `expires in ${String(Math.max(1, Math.ceil(remainingMs / 60_000)))} min`;
}

function grantDurationLabel(seconds: number | null): string {
  if (seconds === null) return "grant";
  if (seconds < 3600) return `${String(Math.round(seconds / 60))} min`;
  if (seconds < 86_400) return `${String(Math.round(seconds / 3600))} h`;
  return `${String(Math.round(seconds / 86_400))} d`;
}

/** Small mono chip used by the scope row (mockup: `.a-scope code`). */
function ScopeChip({ children }: { readonly children: ReactNode }) {
  useUiTranslation();
  return (
    <span className="inline-flex min-w-0 max-w-full items-center gap-1 rounded-md border border-border bg-muted px-1.5 py-0.5 font-mono text-12 text-muted-foreground">
      {children}
    </span>
  );
}

/**
 * Structured scope row: which service is written to, as which agent key,
 * under which approval mode — chips instead of a raw sentence so the
 * three facts scan independently.
 */
function ScopeRow({ block }: { readonly block: ApprovalCardContentBlock }) {
  useUiTranslation();
  return (
    <div className="mt-2.5 flex flex-wrap items-center gap-1.5">
      <span className="type-table-header uppercase tracking-[1px] text-muted-foreground">{translateUi("Scope")}</span>
      <ScopeChip>
        <ServiceIcon slug={`api-${block.service_slug}`} size="2xs" />
        <span className="truncate">{block.service_slug}</span>
      </ScopeChip>
      <ScopeChip>
        <KeyRound className="h-3 w-3 shrink-0" />
        <span className="truncate">{block.agent_key_prefix}</span>
      </ScopeChip>
      <ScopeChip>
        {block.approval_mode === "per_request"
          ? translateUi("per-request approval")
          : translateUi("grant · {{value1}}", { value1: grantDurationLabel(block.grant_duration_sec) })}
      </ScopeChip>
    </div>
  );
}

const DECIDED_STYLE: Record<
  ApprovalDecision,
  { container: string; title: string }
> = {
  approved: {
    container: "border-success/30 bg-success/[0.06]",
    title: "Approved",
  },
  denied: {
    container: "border-destructive/30 bg-destructive/[0.06]",
    title: "Request denied",
  },
  expired: { container: "border-border bg-overlay", title: "Request expired" },
  cancelled: {
    container: "border-border bg-overlay",
    title: "Request cancelled",
  },
};

function DecidedIcon({ decision }: { readonly decision: ApprovalDecision }) {
  useUiTranslation();
  switch (decision) {
    case "approved":
      return <Check className="h-4 w-4 shrink-0 text-success" />;
    case "denied":
      return <X className="h-4 w-4 shrink-0 text-destructive" />;
    case "expired":
      return <Clock3 className="h-4 w-4 shrink-0 text-muted-foreground" />;
    case "cancelled":
      return <Ban className="h-4 w-4 shrink-0 text-muted-foreground" />;
  }
}

function DecidedCard({ block, onRequestAgain, disabled }: { readonly block: ApprovalCardContentBlock; readonly onRequestAgain?: () => void; readonly disabled?: boolean }) {
  useUiTranslation();
  const decision = block.decision;
  if (decision === null) return null;
  const style = DECIDED_STYLE[decision];

  return (
    <section className={`${STATUS_CARD_MAX_W} rounded-xl border p-4 ${style.container}`}>
      <div className="flex flex-wrap items-center gap-2 type-body-strong text-foreground">
        <DecidedIcon decision={decision} />
        <span>{style.title}</span>
        {block.decision_channel && (
          <Badge variant="secondary" className="ml-auto">{translateUi("via ")}{block.decision_channel}
          </Badge>
        )}
      </div>
      <p className="mt-2 text-14 text-muted-foreground">
        {block.body}
      </p>
      {decision !== "approved" && (
        <p className="mt-1.5 text-13 text-text-tertiary">{translateUi("Nothing was sent.")}</p>
      )}
      {decision === "expired" && onRequestAgain && (
        <div className="mt-3">
          <Button type="button" variant="secondary" size="sm" disabled={disabled} onClick={onRequestAgain}>
            <RotateCcw />{translateUi("Ask for a new confirmation")}</Button>
        </div>
      )}
    </section>
  );
}

function SubmittedCard({
  block,
}: {
  readonly block: ApprovalCardContentBlock;
}) {
  useUiTranslation();
  if (!block.decision_submission) return null;
  return (
    <section className={`${STATUS_CARD_MAX_W} rounded-xl border border-warning/30 bg-warning/[0.06] p-4`}>
      <div className="flex items-center gap-2 type-body-strong text-foreground">
        <Loader2 className="h-4 w-4 animate-spin text-warning" />{translateUi("Decision sent")}</div>
      <p className="mt-2 text-14 text-muted-foreground">
        {block.body}
      </p>
      <p className="mt-1.5 text-13 text-text-tertiary">{translateUi("Waiting for the committed ")}{block.decision_submission}{translateUi(" decision.")}</p>
    </section>
  );
}

function LegacyApprovalCard({
  block,
  disabled = false,
  onDecide,
  onRequestAgain,
  reviewPath,
}: {
  readonly block: ApprovalCardContentBlock;
  readonly disabled?: boolean;
  readonly onRequestAgain?: () => void;
  readonly reviewPath?: string;
  readonly onDecide: (approved: boolean) => Promise<void> | void;
}) {
  useUiTranslation();
  const [pendingAction, setPendingAction] = useState<"approve" | "deny" | null>(
    null,
  );
  const [now, setNow] = useState(Date.now());
  const lastClick = useRef(Number.NEGATIVE_INFINITY);

  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(timer);
  }, []);

  async function decide(approved: boolean) {
    const clickedAt = Date.now();
    if (
      Date.parse(block.expires_at) <= clickedAt ||
      block.decision !== null ||
      block.decision_submission != null ||
      disabled ||
      pendingAction !== null ||
      clickedAt - lastClick.current < CLICK_THROTTLE_MS
    ) {
      return;
    }
    lastClick.current = clickedAt;
    setPendingAction(approved ? "approve" : "deny");
    try {
      await onDecide(approved);
    } catch {
      // Delivery failures surface via the mutation's own error toast; the
      // card only needs to re-enable its buttons. Swallowing here also
      // keeps the fire-and-forget click handler from raising an
      // unhandled rejection.
    } finally {
      setPendingAction(null);
    }
  }

  if (block.decision !== null || (!block.decision_submission && Date.parse(block.expires_at) <= now)) {
    return <div className="space-y-2">
      <DecidedCard block={{ ...block, decision: block.decision ?? "expired" }} onRequestAgain={onRequestAgain} disabled={disabled} />
      {reviewPath ? <a className="type-row-title underline" href={reviewPath}>{translateUi("Open Approvals")}</a> : null}
    </div>;
  }
  if (block.decision_submission) {
    return <SubmittedCard block={block} />;
  }

  const remainingMs = new Date(block.expires_at).getTime() - now;
  const expired = remainingMs <= 0;
  const urgent = !expired && remainingMs < URGENT_THRESHOLD_MS;
  const busy = pendingAction !== null;

  return (
    <section className={`${STATUS_CARD_MAX_W} overflow-hidden rounded-xl border border-border bg-card shadow-sm`}>
      <div className="flex items-start gap-3 border-b border-border px-4 py-3.5">
        <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-nyx-secondary-400/30 bg-nyx-secondary-400/10">
          <ShieldAlert className="h-4 w-4 text-nyx-secondary-400" />
        </div>
        <div className="min-w-0 flex-1">
          <h3 className="type-card-title text-foreground">{translateUi("Approval required")}</h3>
          <p className="mt-0.5 text-13 text-muted-foreground">{translateUi("Review the destination and access scope before this action runs.")}</p>
        </div>
        <div className="flex shrink-0 flex-col items-end gap-1.5">
          <Badge variant="accent">{translateUi("Pending")}</Badge>
          <span
            className={`flex items-center gap-1 font-mono ${
              expired || urgent
                ? "type-table-header text-destructive"
                : "type-account-email text-muted-foreground"
            }`}
          >
            <Clock3 className="h-3 w-3" />
            {remainingLabel(remainingMs)}
          </span>
        </div>
      </div>

      <div className="px-4 py-3.5">
        <p className="type-card-title text-foreground">
          {block.body}
        </p>

        <ScopeRow block={block} />
        {reviewPath ? <a className="type-row-title underline" href={reviewPath}>{translateUi("Open Approvals")}</a> : null}

        {block.approval_mode === "grant" && (
          <p className="mt-2.5 text-13 text-muted-foreground">{translateUi("Approving also allows repeat ")}{block.service_slug}{translateUi(" writes for the next ")}{grantDurationLabel(block.grant_duration_sec)}{translateUi(" without asking again.")}</p>
        )}
      </div>

      <div className="flex flex-wrap items-center gap-2 border-t border-border bg-muted px-4 py-3">
        <button
          type="button"
          disabled={busy || disabled}
          onClick={() => void decide(true)}
          className="inline-flex h-8 items-center gap-1.5 rounded-lg border border-success/40 bg-success/15 px-3 type-row-title text-success transition-colors hover:bg-success/20 disabled:cursor-not-allowed disabled:opacity-40 light:text-foreground"
        >
          {pendingAction === "approve" ? (
            <Loader2 className="h-3 w-3 animate-spin light:text-success" />
          ) : (
            <Check className="h-3 w-3 light:text-success" />
          )}{translateUi("Approve and send")}</button>
        <button
          type="button"
          disabled={busy || disabled}
          onClick={() => void decide(false)}
          className="inline-flex h-8 items-center gap-1.5 rounded-lg border border-destructive/40 bg-destructive/15 px-3 type-row-title text-destructive transition-colors hover:bg-destructive/20 disabled:cursor-not-allowed disabled:opacity-40 light:text-foreground"
        >
          {pendingAction === "deny" ? (
            <Loader2 className="h-3 w-3 animate-spin light:text-destructive" />
          ) : (
            <X className="h-3 w-3 light:text-destructive" />
          )}{translateUi("Deny")}</button>
        <span className="ml-auto text-12 text-muted-foreground">{translateUi("Nothing is sent until you decide.")}</span>
      </div>
    </section>
  );
}

export function ApprovalCard(props: Parameters<typeof LegacyApprovalCard>[0]) {
  useUiTranslation();
  return props.block.execution_proposal
    ? <ExecutionPermissionCard requestId={props.block.approval_request_id} proposal={props.block.execution_proposal} disabled={props.disabled} />
    : <LegacyApprovalCard {...props} />;
}
