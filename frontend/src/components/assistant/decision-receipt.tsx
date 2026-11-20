import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { getFormattingLocale } from "@/lib/formatting-locale";
import { formatRelativeTime } from "@/lib/utils";
import { Check, Clock, X } from "lucide-react";
import { ACTION_RISK_LABEL } from "@/lib/assistant/nyxbot-labels";
import type { NyxAgentAcknowledgement } from "@/schemas/assistant-nyxagent";

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
  useUiTranslation();
  const automatic = row.decided_by === "access_mode";
  const bypass = automatic
    ? (
        {
          get webhook() { return translateUi("automation confirmation"); },
          get policy() { return translateUi("policy confirmation"); },
          get org_connection() { return translateUi("organization instruction"); },
          get delegation() { return translateUi("delegation confirmation"); },
          get approval() { return translateUi("service approval"); },
          access_mode: "",
        } as const
      )[row.confirmation?.reason ?? "access_mode"]
    : "";
  const label = retired
    ? translateUi("No longer needed · Full access")
    : automatic
    ? translateUi("Allowed automatically · Full access")
    : row.status === "allowed"
      ? row.decided_by === "user" ? translateUi("You allowed")
        : row.decided_by === "orchestrator" ? translateUi("NyxBot allowed") : translateUi("Allowed")
      : row.status === "denied"
        ? row.decided_by === "user" ? translateUi("You denied")
          : row.decided_by === "orchestrator" ? translateUi("NyxBot denied") : translateUi("Denied")
        : row.status === "used"
          ? translateUi("Used")
          : translateUi("Expired");
  const Glyph =
    row.status === "denied" ? X : retired || row.status === "expired" ? Clock : Check;
  const text = summary ?? row.summary;
  const time = row.decided_at ?? row.created_at;
  return (
    <p
      role="status"
      className="flex items-start gap-1.5 text-13 text-muted-foreground"
    >
      <Glyph aria-hidden="true" className="mt-0.5 h-3 w-3 shrink-0" />
      <span className="min-w-0 flex-1 break-words">
        {label}
        {bypass ? translateUi(" · bypassed {{value1}}", { value1: bypass }) : ""} · {text}
        {row.link ? (
          <>
            {" "}
            <a className="underline" href={row.link}>{translateUi("Inspect or discard proposal")}</a>
          </>
        ) : null}
        {row.confirmation ? ` · ${ACTION_RISK_LABEL[row.confirmation.risk]}` : ""}
        {row.reason?.trim() ? ` · ${row.reason}` : ""}
        <time
          dateTime={time}
          title={new Date(time).toLocaleString(getFormattingLocale())}
          className="text-13 text-text-tertiary"
        >
          {" · "}{formatRelativeTime(time)}
        </time>
      </span>
    </p>
  );
}
