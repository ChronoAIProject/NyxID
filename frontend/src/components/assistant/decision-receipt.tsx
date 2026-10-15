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
  retired = false,
}: {
  readonly acknowledgement: NyxAgentAcknowledgement;
  readonly summary?: string;
  readonly retired?: boolean;
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
  const by = row.decided_by === "orchestrator" ? "NyxBot" : row.decided_by === "user" ? "You" : undefined;
  const label = retired
    ? "No longer needed · Full access"
    : automatic
    ? "Allowed automatically · Full access"
    : row.status === "allowed"
      ? by ? `${by} allowed` : "Allowed"
      : row.status === "denied"
        ? by ? `${by} denied` : "Denied"
        : row.status === "used"
          ? "Used"
          : "Expired";
  const Glyph =
    row.status === "denied" ? X : retired || row.status === "expired" ? Clock : Check;
  const text = summary ?? row.summary;
  const time = row.decided_at ?? row.created_at;
  return (
    <p
      role="status"
      className="ml-[30px] flex flex-wrap items-baseline gap-x-1 px-[7px] text-11 text-muted-foreground"
    >
      <Glyph aria-hidden="true" className="h-3 w-3 shrink-0 self-center" />
      <span>
        {label}
        {bypass ? ` · bypassed ${bypass}` : ""} · {text}
      </span>
      {row.link ? (
        <a className="underline" href={row.link}>
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
