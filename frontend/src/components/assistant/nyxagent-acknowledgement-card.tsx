import { translateUi } from "@/i18n/use-ui-translation";
import { translate as t } from "@/i18n";
import { useAppTranslation } from "@/i18n/use-app-translation";
import { DecisionReceipt } from "./decision-receipt";
import { AuthoredSkillCard } from "./authored-skill-card";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import {
  ACTION_RISK_LABEL,
  confirmationReason,
} from "@/lib/assistant/nyxbot-labels";
import type {
  NyxAgentAcknowledgement,
  NyxAgentAccessMode,
} from "@/schemas/assistant-nyxagent";

function title(row: NyxAgentAcknowledgement): string {
  // A specialist's request that NyxBot decides.
  if (row.decider === "orchestrator") {
    if (row.kind === "service") {
      return t("Allow this agent to use {{service}}?", {
        service: row.service_name ?? row.service_slug ?? t("this service"),
      });
    }
    if (row.kind === "account")
      return t("Allow this agent to read your NyxID account?");
    return t("Confirm: {{summary}}", { summary: row.summary });
  }
  if (row.kind === "service")
    return t("Allow this chat to use {{service}}?", {
      service: row.service_name ?? t("this service"),
    });
  if (row.kind === "account") {
    return t(
      "Allow this chat to manage your NyxID account (keys, channel bots, services, nodes, approval settings)?",
    );
  }
  return t("Confirm: {{summary}}", { summary: row.summary });
}

export function NyxAgentAcknowledgementCard({
  acknowledgement,
  accessMode,
  deciding,
  onDecision,
}: {
  readonly acknowledgement: NyxAgentAcknowledgement;
  readonly accessMode?: NyxAgentAccessMode;
  readonly deciding: boolean;
  readonly onDecision: (
    choice: "allow" | "deny",
    acceptReducedScope?: boolean,
  ) => Promise<unknown>;
}) {
  const { t } = useAppTranslation();
  const [error, setError] = useState<string>();
  const confirmationsBypassed =
    accessMode === "autopilot" && acknowledgement.decider === "user";
  if (acknowledgement.authored_skill)
    return (
      <AuthoredSkillCard
        acknowledgement={acknowledgement}
        confirmationsBypassed={confirmationsBypassed}
        deciding={deciding}
        onDecision={onDecision}
      />
    );
  const label = title(acknowledgement);
  const routed = acknowledgement.decider === "orchestrator";
  if (acknowledgement.status !== "pending" || confirmationsBypassed) {
    return (
      <DecisionReceipt
        acknowledgement={acknowledgement}
        retired={confirmationsBypassed && acknowledgement.status === "pending"}
      />
    );
  }

  async function decide(choice: "allow" | "deny") {
    setError(undefined);
    try {
      await onDecision(choice);
    } catch (cause) {
      setError(
        cause instanceof Error
          ? cause.message
          : t("Could not save your decision. Try again."),
      );
    }
  }

  return (
    <section
      aria-label={label}
      className="ml-[30px] space-y-3 rounded-xl border border-border/50 bg-card p-4"
    >
      {routed ? (
        <p className="text-10 font-medium uppercase tracking-[1.5px] text-text-tertiary">
          {t("Requested from NyxBot")}
        </p>
      ) : null}
      <p className="text-13 font-medium text-foreground">{label}</p>
      {acknowledgement.confirmation ? (
        <p className="text-11 text-muted-foreground">
          <span
            className={
              acknowledgement.confirmation.risk === "destructive"
                ? "font-medium text-destructive"
                : acknowledgement.confirmation.risk === "sensitive"
                  ? "font-medium text-warning"
                  : "font-medium text-foreground"
            }
          >
            {t(ACTION_RISK_LABEL[acknowledgement.confirmation.risk])}
          </span>
          {" · "}
          {confirmationReason(acknowledgement.confirmation)}
        </p>
      ) : null}
      {/*
        Help is selected by kind first and by `routed` (NyxBot decides) after.
        The `skills` and `operations` kinds exist only for a specialist's
        routed request: a specialist you talk to directly raises no card and
        gets a plain missing-access result instead, so naming NyxBot on a routed
        card is accurate. A `skills` card without that routing (an unrouted row
        from an older replica) must not promise that NyxBot resolves it:
        allowing it attaches nothing and wakes nobody. Orchestrator-card help
        (the `routed` branch) is unchanged.
      */}
      <p className="text-12 text-muted-foreground">
        {acknowledgement.kind === "skills"
          ? routed
            ? t(
                "Allowing this proposal asks NyxBot to resolve the exact version. Adding or re-pinning the skill follows NyxBot's permission mode.",
              )
            : t(
                "Allowing records your decision only; it attaches nothing by itself. Add the skill in the agent's Skills editor.",
              )
          : acknowledgement.kind === "operations"
            ? t(
                "This changes the specialist's operations on every thread. Widening follows NyxBot's permission mode.",
              )
            : routed
              ? t(
                  "NyxBot decides this specialist's request against what you asked for.",
                ) +
                " " +
                t(
                  "You can decide it here instead; the agent resumes on its own.",
                )
              : acknowledgement.kind === "action"
                ? t("This confirmation applies once, to this action only.")
                : t("This permission applies to this chat only.")}
        {routed
          ? ""
          : " " +
            t(
              "Allowing lets the assistant continue as soon as it finishes its reply.",
            )}
      </p>
      {error ? (
        <p role="alert" className="text-12 text-destructive">
          {translateUi(error)}
        </p>
      ) : null}
      <div className="flex justify-end gap-2">
        <Button
          variant="outline"
          disabled={deciding}
          onClick={() => void decide("deny")}
        >
          {t("Deny")}
        </Button>
        <Button
          variant="primary"
          disabled={deciding}
          onClick={() => void decide("allow")}
        >
          {t("Allow")}
        </Button>
      </div>
    </section>
  );
}
