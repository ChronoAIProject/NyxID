import { useState } from "react";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { zodResolver } from "@hookform/resolvers/zod";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { useAppForm } from "@/components/ui/form";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  useAgentOperations,
  useSetAgentOperations,
} from "@/hooks/use-agent-operation-scopes";
import {
  operationSelectionSchema,
  type AgentServiceOperations,
  type OperationSelection,
} from "@/schemas/agent-operation-scopes";

export function AgentOperationScopes({
  agentId,
  disabled = false,
}: {
  readonly agentId: string;
  readonly disabled?: boolean;
}) {
  const enabled = useFeature(FEATURE_FLAG.AGENT_OPERATION_SCOPES);
  const query = useAgentOperations(agentId, enabled && !disabled);
  if (!enabled) {
    return (
      <p role="status" className="text-12 text-muted-foreground">
        Operation scope configuration is not enabled yet. Existing operation limits still apply.
      </p>
    );
  }
  return (
    <div className="space-y-3" aria-label="Service operations">
      <p className="text-12 text-muted-foreground">
        Limit each granted service to the operations this specialist needs.
        Guest access and confirmations still apply.
      </p>
      {query.isLoading ? (
        <p className="text-12 text-muted-foreground">Loading operations…</p>
      ) : null}
      {query.error ? (
        <p role="alert" className="text-12 text-destructive">
          Could not load operation scopes.
        </p>
      ) : null}
      {query.data?.map((service) => (
        <ServiceOperationForm
          key={`${service.service_id}:${service.revision}`}
          agentId={agentId}
          service={service}
          disabled={disabled}
        />
      ))}
    </div>
  );
}

export function ServiceOperationForm({
  agentId,
  service,
  disabled = false,
}: {
  readonly agentId: string;
  readonly service: AgentServiceOperations;
  readonly disabled?: boolean;
}) {
  const mutation = useSetAgentOperations(agentId);
  const [search, setSearch] = useState("");
  const [error, setError] = useState<string>();
  const form = useAppForm<OperationSelection>({
    resolver: zodResolver(operationSelectionSchema),
    defaultValues: {
      expected_revision: service.revision,
      all_operations: service.all_operations,
      endpoint_ids: service.endpoint_ids,
      rules: service.rules,
    },
  });
  const all = form.watch("all_operations");
  const selected = form.watch("endpoint_ids");
  const rules = form.watch("rules");
  const operations = service.operations.filter((op) =>
    `${op.method} ${op.path} ${op.summary ?? ""}`
      .toLowerCase()
      .includes(search.toLowerCase()),
  );
  async function save(selection: OperationSelection) {
    setError(undefined);
    try {
      const next = selection.all_operations
        ? { ...selection, endpoint_ids: [], rules: [] }
        : selection;
      await mutation.mutateAsync({
        serviceId: service.service_id,
        selection: next,
      });
      form.reset(next);
    } catch (cause) {
      setError(
        cause instanceof Error
          ? cause.message
          : "Could not save operations. Reload and try again.",
      );
    }
  }
  return (
    <form
      aria-label={`${service.service_name} operations`}
      onSubmit={form.handleSubmit(save)}
      className="space-y-3 rounded-xl border border-border/50 bg-card p-3"
    >
      <div className="flex items-center justify-between gap-2">
        <span className="text-13 font-semibold">
          {service.service_name}
        </span>
        <Badge variant="secondary">Revision {service.revision}</Badge>
      </div>
      <Select
        value={all ? "all" : "selected"}
        disabled={disabled}
        onValueChange={(value) =>
          form.setValue("all_operations", value === "all")
        }
      >
        <SelectTrigger aria-label={`${service.service_name} operation access`}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="all">All operations</SelectItem>
          <SelectItem value="selected">Selected operations</SelectItem>
        </SelectContent>
      </Select>
      {!all ? (
        <>
          {service.operations.length ? (
            <>
              <Input
                aria-label={`Search ${service.service_name} operations`}
                placeholder="Search method, path or summary"
                value={search}
                onChange={(event) => setSearch(event.target.value)}
              />
              <div className="flex items-center justify-between gap-2">
                <span className="text-11 text-muted-foreground">
                  {selected.length} selected
                </span>
                <Button
                  type="button"
                  size="sm"
                  variant="ghost"
                  disabled={disabled}
                  onClick={() =>
                    form.setValue(
                      "endpoint_ids",
                      service.operations
                        .filter((op) => op.read_only && !op.changes_existing)
                        .map((op) => op.endpoint_id),
                    )
                  }
                >
                  Select all reads
                </Button>
              </div>
              <div className="assistant-scrollbar max-h-64 space-y-2 overflow-auto">
                {operations.map((operation) => (
                  <label
                    key={operation.endpoint_id}
                    className="flex cursor-pointer items-start gap-2 rounded-lg border border-border/50 p-2 text-12"
                  >
                    <Checkbox
                      checked={selected.includes(operation.endpoint_id)}
                      disabled={disabled}
                      aria-label={`${operation.method} ${operation.path}`}
                      onCheckedChange={(checked) =>
                        form.setValue(
                          "endpoint_ids",
                          checked
                            ? [...selected, operation.endpoint_id]
                            : selected.filter(
                                (id) => id !== operation.endpoint_id,
                              ),
                        )
                      }
                    />
                    <span className="min-w-0 space-y-1">
                      <span className="block break-all font-mono">
                        {operation.method} {operation.path}
                      </span>
                      <span className="block text-muted-foreground">
                        {operation.summary}
                      </span>
                      <span className="flex gap-1">
                        {operation.read_only ? (
                          <Badge variant="success">Read only</Badge>
                        ) : null}
                        {operation.changes_existing ? (
                          <Badge variant="warning">Changes existing</Badge>
                        ) : null}
                      </span>
                    </span>
                  </label>
                ))}
                {!operations.length ? (
                  <p className="text-12 text-muted-foreground">
                    No matching operations.
                  </p>
                ) : null}
              </div>
            </>
          ) : service.allows_explicit_rules ? (
            <>
              <p className="text-12 text-muted-foreground">
                No catalog operations. Add exact methods and paths; a variable
                such as {"{id}"} matches one segment.
              </p>
              {rules.map((rule, index) => (
                <div key={index} className="flex gap-2">
                  <Select
                    value={rule.method}
                    disabled={disabled}
                    onValueChange={(method) =>
                      form.setValue(
                        `rules.${index}.method`,
                        method as typeof rule.method,
                      )
                    }
                  >
                    <SelectTrigger
                      className="w-28"
                      aria-label={`Rule ${index + 1} method`}
                    >
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {[
                        "GET",
                        "HEAD",
                        "OPTIONS",
                        "POST",
                        "PUT",
                        "PATCH",
                        "DELETE",
                      ].map((method) => (
                        <SelectItem key={method} value={method}>
                          {method}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <Input
                    aria-label={`Rule ${index + 1} path`}
                    {...form.register(`rules.${index}.path_template`)}
                    disabled={disabled}
                  />
                  <Button
                    type="button"
                    size="sm"
                    disabled={disabled}
                    onClick={() =>
                      form.setValue(
                        "rules",
                        rules.filter((_, i) => i !== index),
                      )
                    }
                  >
                    Remove
                  </Button>
                </div>
              ))}
              <Button
                type="button"
                size="sm"
                disabled={disabled || rules.length >= 256}
                onClick={() =>
                  form.setValue("rules", [
                    ...rules,
                    { method: "GET", path_template: "/" },
                  ])
                }
              >
                Add rule
              </Button>
            </>
          ) : (
            <p className="text-12 text-muted-foreground">
              No active catalog operations.
            </p>
          )}
          {selected.some(
            (id) => !service.operations.some((op) => op.endpoint_id === id),
          ) ? (
            <p role="status" className="text-12 text-muted-foreground">
              Some selected operations are unavailable.{" "}
              <Button
                type="button"
                size="sm"
                onClick={() =>
                  form.setValue(
                    "endpoint_ids",
                    selected.filter((id) =>
                      service.operations.some((op) => op.endpoint_id === id),
                    ),
                  )
                }
                disabled={disabled}
              >
                Remove unavailable selections
              </Button>
            </p>
          ) : null}
          <p className="text-11 text-muted-foreground">
            An empty selection blocks every operation.
          </p>
        </>
      ) : null}
      {Object.keys(form.formState.errors).length ? (
        <p role="alert" className="text-12 text-destructive">
          Check the selection: paths must start with / and at most 256
          operations may be selected.
        </p>
      ) : null}
      {error ? (
        <p role="alert" className="text-12 text-destructive">
          {error}
        </p>
      ) : null}
      <div className="flex justify-end gap-2">
        <Button
          type="button"
          size="sm"
          disabled={disabled || !form.formState.isDirty}
          onClick={() => {
            form.reset();
            setError(undefined);
          }}
        >
          Cancel
        </Button>
        <Button
          type="submit"
          variant="primary"
          size="sm"
          disabled={disabled || !form.formState.isDirty}
          isLoading={mutation.isPending}
        >
          Save operations
        </Button>
      </div>
    </form>
  );
}
