import { Button } from "@/components/ui/button";
import { useDecideSharedConsent } from "@/hooks/use-shared-agents";
import { useCardTranslation } from "./card-language";
import type { SharedConsent } from "@/schemas/assistant-sharing";
import { toast } from "sonner";
import { DecisionReview } from "./decision-review";

/** The same exact, recipient-owned consent choices as the shared-agent editor. */
export function SharedConsentDecision({ agentId, request }: {
  readonly agentId: string;
  readonly request: SharedConsent;
}) {
  const { t: translateUi } = useCardTranslation();
  const decide = useDecideSharedConsent(agentId);
  function save(decision: "once" | "always" | "skip") {
    void decide.mutateAsync({ consent: request.id, decision }).then(
      () => toast.success(translateUi("Choice saved. Continue the chat when ready.")),
      () => undefined,
    );
  }
  const actions = <div className="flex flex-wrap justify-end gap-2">
    {(["skip", "always", "once"] as const).map((decision) => <Button key={decision} variant={decision === "once" ? "primary" : "outline"} disabled={decide.isPending} onClick={() => save(decision)}>
      {decision === "once" ? translateUi("Allow once") : decision === "always" ? translateUi("Always") : translateUi("Skip")}
    </Button>)}
  </div>;
  return <DecisionReview title={translateUi("Connection consent")} busy={decide.isPending} onDeny={() => save("skip")} denyLabel={translateUi("Skip")} actions={actions}
    summary={<>
      <p>{request.connection_name ?? request.connection_id}</p>
      <p className="line-clamp-2 break-words">{request.operation}</p>
      {decide.error ? <p role="alert" className="text-destructive">{decide.error.message}</p> : null}
    </>}
  ><div className="space-y-3">
    <p className="type-card-title">{translateUi("Use your connection")}</p>
    <p className="type-row-title">{request.connection_name ?? request.connection_id}</p>
    <p className="type-body break-words">{request.operation}</p>
    <p className="type-secondary text-muted-foreground">{translateUi("Your connection: ")}{request.connection_name ?? request.connection_id}{translateUi(". Model and metered-service usage bills you. ")}</p>
    <p className="type-secondary text-muted-foreground">{translateUi("Applies to this exact operation, or this turn’s stateless model calls. Always remembers this scope until you revoke it or the agent or connection changes. Your connection is never shared with the owner.")}</p>
    {request.audience ? <p className="type-secondary text-muted-foreground">{request.audience}</p> : null}
    {decide.error ? <p role="alert" className="type-body text-destructive">{decide.error.message}</p> : null}
  </div></DecisionReview>;
}
