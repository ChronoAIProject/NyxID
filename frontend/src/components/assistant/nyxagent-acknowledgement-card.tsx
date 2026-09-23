import { useState } from "react";
import { Button } from "@/components/ui/button";
import type { NyxAgentAcknowledgement } from "@/schemas/assistant-nyxagent";

function title(row: NyxAgentAcknowledgement): string {
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
  if (acknowledgement.status !== "pending") {
    return (
      <p role="status" className="ml-[30px] px-[7px] text-[11px] text-muted-foreground">
        {statusLabel[acknowledgement.status]} · {label}
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
      <p className="text-[13px] font-medium text-foreground">{label}</p>
      <p className="text-[12px] text-muted-foreground">
        {acknowledgement.kind === "action"
          ? "This confirmation applies once, to this action only."
          : "This permission applies to this chat only."}
        {" Allowing lets the assistant continue as soon as it finishes its reply."}
      </p>
      {error ? (
        <p role="alert" className="text-[12px] text-destructive">
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
