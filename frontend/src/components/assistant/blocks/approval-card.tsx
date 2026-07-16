import { useEffect, useRef, useState, type ReactNode } from "react";
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
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
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
  return (
    <span className="inline-flex min-w-0 items-center gap-1 rounded-md border border-hairline bg-overlay-strong px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground">
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
  return (
    <div className="mt-2.5 flex flex-wrap items-center gap-1.5">
      <span className="text-[10px] font-semibold uppercase tracking-[1px] text-text-tertiary">
        Scope
      </span>
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
          ? "per-request approval"
          : `grant · ${grantDurationLabel(block.grant_duration_sec)}`}
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
    title: "Approved and sent",
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

function DecidedCard({ block }: { readonly block: ApprovalCardContentBlock }) {
  const decision = block.decision;
  if (decision === null) return null;
  const style = DECIDED_STYLE[decision];

  return (
    <section className={`rounded-xl border p-4 ${style.container}`}>
      <div className="flex flex-wrap items-center gap-2 text-[12px] font-semibold text-foreground">
        <DecidedIcon decision={decision} />
        {style.title}
        {block.decision_channel && (
          <Badge variant="secondary" className="ml-auto">
            via {block.decision_channel}
          </Badge>
        )}
      </div>
      <p className="mt-2 text-[12px] leading-relaxed text-muted-foreground">
        {block.body}
      </p>
      {decision !== "approved" && (
        <p className="mt-1.5 text-[11px] text-text-tertiary">
          Nothing was sent.
        </p>
      )}
      {decision === "expired" && (
        <div className="mt-3">
          <Tooltip>
            <TooltipTrigger asChild>
              <Button type="button" variant="secondary" size="sm">
                <RotateCcw />
                Request again
              </Button>
            </TooltipTrigger>
            <TooltipContent>Wired in the API pass</TooltipContent>
          </Tooltip>
        </div>
      )}
    </section>
  );
}

export function ApprovalCard({
  block,
  onDecide,
}: {
  readonly block: ApprovalCardContentBlock;
  readonly onDecide: (approved: boolean) => Promise<void> | void;
}) {
  const [pendingAction, setPendingAction] = useState<
    "approve" | "deny" | null
  >(null);
  const [now, setNow] = useState(Date.now());
  const lastClick = useRef(Number.NEGATIVE_INFINITY);

  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(timer);
  }, []);

  async function decide(approved: boolean) {
    const clickedAt = Date.now();
    if (
      block.decision !== null ||
      pendingAction !== null ||
      clickedAt - lastClick.current < CLICK_THROTTLE_MS
    ) {
      return;
    }
    lastClick.current = clickedAt;
    setPendingAction(approved ? "approve" : "deny");
    try {
      await onDecide(approved);
    } finally {
      setPendingAction(null);
    }
  }

  if (block.decision !== null) {
    return <DecidedCard block={block} />;
  }

  const remainingMs = new Date(block.expires_at).getTime() - now;
  const expired = remainingMs <= 0;
  const urgent = !expired && remainingMs < URGENT_THRESHOLD_MS;
  const busy = pendingAction !== null;

  return (
    <section className="rounded-xl border border-warning/30 bg-warning/[0.06] p-4">
      <div className="flex flex-wrap items-center gap-2">
        <ShieldAlert className="h-4 w-4 text-warning" />
        <h3 className="text-[12px] font-semibold text-warning">
          Approval required
        </h3>
        <span
          className={`ml-auto flex items-center gap-1 font-mono text-[10px] ${
            expired || urgent
              ? "font-semibold text-destructive"
              : "text-muted-foreground"
          }`}
        >
          <Clock3 className="h-3 w-3" />
          {remainingLabel(remainingMs)}
        </span>
      </div>

      <p className="mt-3 text-[13px] font-medium leading-relaxed text-foreground">
        {block.body}
      </p>

      <ScopeRow block={block} />

      {block.approval_mode === "grant" && (
        <p className="mt-2 text-[11px] leading-relaxed text-muted-foreground">
          Approving also allows repeat {block.service_slug} writes for the next{" "}
          {grantDurationLabel(block.grant_duration_sec)} without asking again.
        </p>
      )}

      <div className="mt-4 flex flex-wrap items-center gap-2">
        <button
          type="button"
          disabled={busy}
          onClick={() => void decide(true)}
          className="inline-flex h-8 items-center gap-1.5 rounded-lg border border-success/30 bg-success/10 px-3 text-[12px] font-medium text-success transition-colors hover:bg-success/15 disabled:cursor-not-allowed disabled:opacity-40 light:text-foreground"
        >
          {pendingAction === "approve" ? (
            <Loader2 className="h-3 w-3 animate-spin light:text-success" />
          ) : (
            <Check className="h-3 w-3 light:text-success" />
          )}
          Approve and send
        </button>
        <button
          type="button"
          disabled={busy}
          onClick={() => void decide(false)}
          className="inline-flex h-8 items-center gap-1.5 rounded-lg border border-destructive/30 bg-destructive/10 px-3 text-[12px] font-medium text-destructive transition-colors hover:bg-destructive/15 disabled:cursor-not-allowed disabled:opacity-40 light:text-foreground"
        >
          {pendingAction === "deny" ? (
            <Loader2 className="h-3 w-3 animate-spin light:text-destructive" />
          ) : (
            <X className="h-3 w-3 light:text-destructive" />
          )}
          Deny
        </button>
        <span className="ml-auto text-[10px] text-text-tertiary">
          Nothing is sent until you decide.
        </span>
      </div>
    </section>
  );
}
