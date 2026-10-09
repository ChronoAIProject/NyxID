import { ListChecks } from "lucide-react";
import { OperationSelectionForm } from "@/components/assistant/agent-operation-scopes";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { useFeature } from "@/hooks/use-feature-flag";
import {
  useApiKeyOperations,
  useSetApiKeyOperations,
} from "@/hooks/use-api-key-operations";
import { ApiError } from "@/lib/api-client";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import type { AgentServiceOperations } from "@/schemas/agent-operation-scopes";

/** Limit an Agent Key to selected operations within each service it may use. */
export function OperationScopeCard({
  keyId,
  canWrite,
}: {
  readonly keyId: string;
  readonly canWrite: boolean;
}) {
  const enabled = useFeature(FEATURE_FLAG.AGENT_OPERATION_SCOPES);
  const query = useApiKeyOperations(keyId, enabled);
  // Scheduled and permission-bound keys have their own authority models;
  // the server refuses them, so there is nothing to configure here.
  if (!enabled || (query.error instanceof ApiError && query.error.status === 400)) {
    return null;
  }
  return (
    <Card className="md:col-span-2">
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <ListChecks className="h-4 w-4" aria-hidden />
          Service operations
        </CardTitle>
        <CardDescription>
          Limit this key to the operations it needs within each service.
          Approvals still apply to every call.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        {query.isLoading ? (
          <p className="text-12 text-muted-foreground">Loading operations…</p>
        ) : null}
        {query.error ? (
          <p role="alert" className="text-12 text-destructive">
            Could not load operations.
          </p>
        ) : null}
        {query.data && !query.data.length ? (
          <p className="text-12 text-muted-foreground">
            This key has no services to limit yet.
          </p>
        ) : null}
        {query.data?.map((service) => (
          <KeyServiceOperations
            key={`${service.service_id}:${service.revision}`}
            keyId={keyId}
            service={service}
            disabled={!canWrite}
          />
        ))}
      </CardContent>
    </Card>
  );
}

function KeyServiceOperations({
  keyId,
  service,
  disabled,
}: {
  readonly keyId: string;
  readonly service: AgentServiceOperations;
  readonly disabled: boolean;
}) {
  const mutation = useSetApiKeyOperations(keyId);
  return (
    <OperationSelectionForm
      service={service}
      disabled={disabled}
      isSaving={mutation.isPending}
      save={(selection) =>
        mutation.mutateAsync({ serviceId: service.service_id, selection })
      }
    />
  );
}
