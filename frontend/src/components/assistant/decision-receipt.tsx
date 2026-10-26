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
          policy: "policy confirmation",
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
      className="flex items-start gap-1.5 text-11 text-muted-foreground"
    >
      <Glyph aria-hidden="true" className="mt-0.5 h-3 w-3 shrink-0" />
      <span className="min-w-0 flex-1 break-words">
        {label}
        {bypass ? ` · bypassed ${bypass}` : ""} · {text}
        {row.link ? (
          <>
            {" "}
            <a className="underline" href={row.link}>
              Inspect or discard proposal
            </a>
          </>
        ) : null}
        {row.confirmation ? ` · ${ACTION_RISK_LABEL[row.confirmation.risk]}` : ""}
        {row.reason?.trim() ? ` · ${row.reason}` : ""}
        <time
          dateTime={time}
          title={new Date(time).toLocaleString()}
          className="text-11 text-text-tertiary"
        >
          {" · "}{relativeTime(time)}
        </time>
      </span>
    </p>
  );
}
