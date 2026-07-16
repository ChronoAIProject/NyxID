import { useEffect, useRef, useState } from "react";
import { Check, Clock3, ShieldAlert, X } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import type { ApprovalCardContentBlock } from "@/types/assistant";

const CLICK_THROTTLE_MS = 750;

function expiryLabel(expiresAt: string, now: number): string {
  const remaining = new Date(expiresAt).getTime() - now;
  if (remaining <= 0) return "expired";
  return `expires in ${String(Math.max(1, Math.ceil(remaining / 60_000)))} min`;
}

export function ApprovalCard({
  block,
  onDecide,
}: {
  readonly block: ApprovalCardContentBlock;
  readonly onDecide: (approved: boolean) => Promise<void> | void;
}) {
  const [pending, setPending] = useState(false);
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
      pending ||
      clickedAt - lastClick.current < CLICK_THROTTLE_MS
    ) {
      return;
    }
    lastClick.current = clickedAt;
    setPending(true);
    try {
      await onDecide(approved);
    } finally {
      setPending(false);
    }
  }

  if (block.decision !== null) {
    const approved = block.decision === "approved";
    return (
      <section
        className={`rounded-xl border p-4 ${
          approved
            ? "border-success/30 bg-success/[0.06]"
            : block.decision === "denied"
              ? "border-destructive/30 bg-destructive/[0.06]"
              : "border-border bg-overlay"
        }`}
      >
        <div className="flex items-center gap-2 text-[12px] font-semibold text-foreground">
          {approved ? (
            <Check className="h-4 w-4 text-success" />
          ) : (
            <X className="h-4 w-4 text-destructive" />
          )}
          {approved
            ? "Approved and sent"
            : block.decision === "denied"
              ? "Request denied"
              : "Request closed"}
          {block.decision_channel && (
            <Badge variant="secondary" className="ml-auto">
              via {block.decision_channel}
            </Badge>
          )}
        </div>
        <p className="mt-2 text-[12px] leading-relaxed text-muted-foreground">
          {block.body}
        </p>
      </section>
    );
  }

  return (
    <section className="rounded-xl border border-warning/30 bg-warning/[0.06] p-4">
      <div className="flex flex-wrap items-center gap-2">
        <ShieldAlert className="h-4 w-4 text-warning" />
        <h3 className="text-[12px] font-semibold text-warning">
          Approval required
        </h3>
        <span className="ml-auto flex items-center gap-1 font-mono text-[10px] text-muted-foreground">
          <Clock3 className="h-3 w-3" />
          {expiryLabel(block.expires_at, now)}
        </span>
      </div>
      <p className="mt-3 text-[13px] leading-relaxed text-foreground">
        {block.body}
      </p>
      <p className="mt-1 break-words font-mono text-[10px] text-text-tertiary">
        Scope: {block.service_slug} - agent key {block.agent_key_prefix} -{" "}
        {block.approval_mode.replaceAll("_", " ")}
      </p>
      <div className="mt-4 flex flex-wrap items-center gap-2">
        <button
          type="button"
          disabled={pending}
          onClick={() => void decide(true)}
          className="inline-flex h-8 items-center gap-1.5 rounded-lg border border-success/30 bg-success/10 px-3 text-[12px] font-medium text-success transition-colors hover:bg-success/15 disabled:cursor-not-allowed disabled:opacity-40 light:text-foreground"
        >
          <Check className="h-3 w-3 light:text-success" />
          Approve and send
        </button>
        <button
          type="button"
          disabled={pending}
          onClick={() => void decide(false)}
          className="inline-flex h-8 items-center gap-1.5 rounded-lg border border-destructive/30 bg-destructive/10 px-3 text-[12px] font-medium text-destructive transition-colors hover:bg-destructive/15 disabled:cursor-not-allowed disabled:opacity-40 light:text-foreground"
        >
          <X className="h-3 w-3 light:text-destructive" />
          Deny
        </button>
      </div>
    </section>
  );
}
