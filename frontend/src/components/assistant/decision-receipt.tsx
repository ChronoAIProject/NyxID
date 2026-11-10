import { useAppTranslation } from "@/i18n/use-app-translation";
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
  const { t, i18n } = useAppTranslation();
  const automatic = row.decided_by === "access_mode";
  const bypass = automatic
    ? (
        {
          webhook: t("automation confirmation"),
          policy: t("machine / saved-login confirmation"),
          approval: t("service approval"),
          access_mode: "",
        } as const
      )[row.confirmation?.reason ?? "access_mode"]
    : "";
  const by =
    row.decided_by === "orchestrator"
      ? "NyxBot"
      : row.decided_by === "user"
        ? t("You")
        : undefined;
  const label = retired
    ? t("No longer needed · Full access")
    : automatic
      ? t("Allowed automatically · Full access")
      : row.status === "allowed"
        ? by
          ? t("{{name}} allowed", { name: by })
          : t("Allowed")
        : row.status === "denied"
          ? by
            ? t("{{name}} denied", { name: by })
            : t("Denied")
          : row.status === "used"
            ? t("Used")
            : t("Expired");
  const Glyph =
    row.status === "denied"
      ? X
      : retired || row.status === "expired"
        ? Clock
        : Check;
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
        {bypass
          ? t(" · {{value}}", { value: t("bypassed {{confirmation}}", { confirmation: bypass }) })
          : ""}{" "}
        · {text}
      </span>
      {row.link ? (
        <a className="underline" href={row.link}>
          {t("Inspect or discard proposal")}
        </a>
      ) : null}
      {row.confirmation ? (
        <span>· {t(ACTION_RISK_LABEL[row.confirmation.risk])}</span>
      ) : null}
      {row.reason?.trim() ? <span>· {row.reason}</span> : null}
      <time
        dateTime={time}
        title={new Date(time).toLocaleString(i18n.language)}
        className="text-11 text-text-tertiary"
      >
        · {formatRelativeTime(time)}
      </time>
    </p>
  );
}
