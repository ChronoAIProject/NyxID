import { useState } from "react";
import { Button } from "@/components/ui/button";
import type { NyxAgentAcknowledgement } from "@/schemas/assistant-nyxagent";

function title(row: NyxAgentAcknowledgement): string {
  // A specialist's request that NyxBot decides.
  if (row.decider === "orchestrator") {
    if (row.kind === "service") {
      return `Allow this agent to use ${row.service_name ?? row.service_slug ?? "this service"}?`;
    }
    if (row.kind === "account") return "Allow this agent to read your NyxID account?";
    return `Confirm: ${row.summary}`;
  }
  if (row.kind === "service")
    return `Allow this chat to use ${row.service_name ?? "this service"}?`;
  if (row.kind === "account") {
    return (
      "Allow this chat to manage your NyxID account " +
      "(keys, channel bots, services, nodes, approval settings)?"
    );
  }
  return `Confirm: ${row.summary}`;
}

const statusLabel = {
  pending: "Pending",
  allowed: "Allowed",
  denied: "Denied",
  expired: "Expired",
  used: "Used",
};

/** "Allowed by NyxBot: <reason>" for decisions whose decider is recorded. */
function decisionLabel(row: NyxAgentAcknowledgement): string {
  const status = statusLabel[row.status];
  if ((row.status !== "allowed" && row.status !== "denied") || !row.decided_by) return status;
  const by = row.decided_by === "orchestrator" ? "NyxBot" : "you";
  const reason = row.reason?.trim();
  return reason ? `${status} by ${by}: ${reason}` : `${status} by ${by}`;
}

export function NyxAgentAcknowledgementCard({
  acknowledgement,
  deciding,
  onDecision,
}: {
  readonly acknowledgement: NyxAgentAcknowledgement;
  readonly deciding: boolean;
  readonly onDecision: (choice: "allow" | "deny") => Promise<unknown>;
}) {
  const [error, setError] = useState<string>();
  const label = title(acknowledgement);
  const routed = acknowledgement.decider === "orchestrator";
  if (acknowledgement.status !== "pending") {
    return (
      <p role="status" className="ml-[30px] px-[7px] text-11 text-muted-foreground">
        {decisionLabel(acknowledgement)} · {label}
      </p>
    );
  }

  async function decide(choice: "allow" | "deny") {
    setError(undefined);
    try {
      await onDecision(choice);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Could not save your decision. Try again.");
    }
  }

  return (
    <section
      aria-label={label}
      className="ml-[30px] space-y-3 rounded-xl border border-border/50 bg-card p-4"
    >
      {routed ? (
        <p className="text-10 font-medium uppercase tracking-[1.5px] text-text-tertiary">
          Requested from NyxBot
        </p>
      ) : null}
      <p className="text-13 font-medium text-foreground">{label}</p>
      <p className="text-12 text-muted-foreground">
        {acknowledgement.kind === "skills"
          ? "This proposal does not attach content. NyxBot must request your confirmation before adding or re-pinning skills."
          : acknowledgement.kind === "operations"
          ? "This changes the specialist's operations on every thread. Widening requires your confirmation."
          : routed
          ? "NyxBot decides this specialist's request against what you asked for. " +
            "You can decide it here instead; the agent resumes on its own."
          : acknowledgement.kind === "action"
            ? "This confirmation applies once, to this action only."
            : "This permission applies to this chat only."}
        {routed ? "" : " Allowing lets the assistant continue as soon as it finishes its reply."}
      </p>
      {error ? (
        <p role="alert" className="text-12 text-destructive">
          {error}
        </p>
      ) : null}
      <div className="flex justify-end gap-2">
        <Button variant="outline" disabled={deciding} onClick={() => void decide("deny")}>
          Deny
        </Button>
        <Button variant="primary" disabled={deciding} onClick={() => void decide("allow")}>
          Allow
        </Button>
      </div>
    </section>
  );
}
