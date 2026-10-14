import { Check, Clock, X } from "lucide-react";
import { ACTION_RISK_LABEL } from "@/lib/assistant/nyxbot-labels";
import type { NyxAgentAcknowledgement } from "@/schemas/assistant-nyxagent";

function relativeTime(value: string): string {
  const minutes = Math.max(
    0,
    Math.floor((Date.now() - Date.parse(value)) / 60_000),
  );
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes}m ago`;
  if (minutes < 1440) return `${Math.floor(minutes / 60)}h ago`;
  return `${Math.floor(minutes / 1440)}d ago`;
}

/** A durable decision in the transcript, without interactive card chrome. */
export function DecisionReceipt({
  acknowledgement: row,
  summary,
}: {
  readonly acknowledgement: NyxAgentAcknowledgement;
  readonly summary?: string;
}) {
  const automatic = row.decided_by === "access_mode";
  const bypass = automatic
    ? (
        {
          webhook: "automation confirmation",
          policy: "machine / saved-login confirmation",
          approval: "service approval",
          access_mode: "",
        } as const
      )[row.confirmation?.reason ?? "access_mode"]
    : "";
  const by = row.decided_by === "orchestrator" ? "NyxBot" : "You";
  const label = automatic
    ? "Allowed automatically · Full access"
    : row.status === "allowed"
      ? `${by} allowed`
      : row.status === "denied"
        ? `${by} denied`
        : row.status === "used"
          ? "Used"
          : "Expired";
  const Glyph =
    row.status === "denied" ? X : row.status === "expired" ? Clock : Check;
  const text = summary ?? row.summary;
  const proposal = /\. Inspect or discard: (https?:\/\/\S+)$/.exec(text);
  const shownSummary = proposal ? text.slice(0, proposal.index) : text;
  const time = row.decided_at ?? row.created_at;
  return (
    <p
      role="status"
      className="ml-[30px] flex flex-wrap items-baseline gap-x-1 px-[7px] text-11 text-muted-foreground"
    >
      <Glyph aria-hidden="true" className="h-3 w-3 shrink-0 self-center" />
      <span>
        {label}
        {bypass ? ` · bypassed ${bypass}` : ""} · {shownSummary}
      </span>
      {proposal ? (
        <a className="underline" href={proposal[1]}>
          Inspect or discard proposal
        </a>
      ) : null}
      {row.confirmation ? (
        <span>· {ACTION_RISK_LABEL[row.confirmation.risk]}</span>
      ) : null}
      {row.reason?.trim() ? <span>· {row.reason}</span> : null}
      <time
        dateTime={time}
        title={new Date(time).toLocaleString()}
        className="text-11 text-text-tertiary"
      >
        · {relativeTime(time)}
      </time>
    </p>
  );
}
