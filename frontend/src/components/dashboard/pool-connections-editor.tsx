import { useEffect, useState } from "react";
import { useWatch, type UseFormReturn } from "react-hook-form";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { usePoolCandidates, usePoolHealth } from "@/hooks/use-pools";
import type {
  CreateServicePoolInput,
  PoolCandidate,
  ServicePool,
  ServicePoolMember,
} from "@/schemas/pools";
import { NumberInput, Toggle } from "./pool-controls";
import { bindingLabel, reason } from "./pool-labels";
import { PoolConnectionPicker } from "./pool-connection-picker";
import { PoolOperationCheck, type PoolOperation } from "./pool-operation-check";

function newMember(id: string): ServicePoolMember {
  return {
    user_service_id: id,
    weight: 1,
    enabled: true,
    priority: 0,
    model: null,
    same_api_compatible: false,
  };
}

export function PoolConnectionsEditor({
  form,
  pool,
  orgId,
  operation,
  setOperation,
}: {
  form: UseFormReturn<CreateServicePoolInput>;
  pool?: ServicePool;
  orgId?: string;
  operation: PoolOperation | null;
  setOperation: (operation: PoolOperation | null) => void;
}) {
  const values = useWatch({ control: form.control });
  const members = values.members ?? [];
  const priority = values.strategy === "priority";
  const aiChat = priority && values.member_contract === "ai_chat";
  const weighted =
    values.strategy === "weighted" ||
    (priority && values.tier_balance === "weighted");
  const [search, setSearch] = useState("");
  const [settledSearch, setSettledSearch] = useState("");
  useEffect(() => {
    const timer = setTimeout(() => setSettledSearch(search), 300);
    return () => clearTimeout(timer);
  }, [search]);
  const inspection = {
    poolId: pool?.id,
    orgId,
    contract: values.member_contract,
    strategy: values.strategy,
    checkOperation: !aiChat && operation !== null,
    method: aiChat ? undefined : operation?.method,
    path: aiChat ? undefined : operation?.path,
    declaredPeerIds: members
      .filter((m) => m.same_api_compatible)
      .map((m) => m.user_service_id!),
    peerIds: members.map((m) => m.user_service_id!).filter(Boolean),
  };
  const candidates = usePoolCandidates({
    ...inspection,
    search: settledSearch,
  });
  const selected = usePoolCandidates(
    { ...inspection, selectedOnly: true },
    members.length > 0,
  );
  const rows = candidates.data?.pages.flatMap((page) => page.candidates) ?? [];
  const selectedRows = new Map(
    (selected.data?.pages.flatMap((page) => page.candidates) ?? []).map(
      (row) => [row.user_service_id, row],
    ),
  );
  const [selectedLabels, setSelectedLabels] = useState<
    Record<string, PoolCandidate>
  >({});
  const savedMembers = usePoolHealth({
    poolId: pool?.id,
    contract: pool?.member_contract,
    checkOperation: false,
  });
  const savedRows = new Map(
    savedMembers.data?.candidates.map((row) => [row.user_service_id, row]) ??
      [],
  );
  function setMember(id: string, patch: Partial<ServicePoolMember>) {
    form.setValue(
      "members",
      form
        .getValues("members")
        .map((m) => (m.user_service_id === id ? { ...m, ...patch } : m)),
    );
  }
  function toggleMember(row: PoolCandidate) {
    const current = form.getValues("members");
    if (
      current.some((member) => member.user_service_id === row.user_service_id)
    ) {
      form.setValue(
        "members",
        current.filter(
          (member) => member.user_service_id !== row.user_service_id,
        ),
      );
      return;
    }
    if (
      current.length >= 50 ||
      (!row.eligible && row.reason !== "compatibility_declaration_required")
    )
      return;
    setSelectedLabels((labels) => ({ ...labels, [row.user_service_id]: row }));
    const nextPriority =
      priority && current.length
        ? Math.min(
            4294967295,
            Math.max(...current.map((member) => member.priority ?? 0)) + 1,
          )
        : 0;
    form.setValue("members", [
      ...current,
      { ...newMember(row.user_service_id), priority: nextPriority },
    ]);
  }
  const orderedMembers = members
    .map((member, index) => ({ member, index }))
    .sort((a, b) =>
      priority
        ? (a.member.priority ?? 0) - (b.member.priority ?? 0)
        : a.index - b.index,
    );
  return (
    <section className="space-y-3" aria-label="Pool connections">
      <div>
        <h3 className="text-[13px] font-semibold">
          {priority ? "Connection order" : "Connections"}
        </h3>
        <p className="mt-1 text-[12px] text-muted-foreground">
          {priority
            ? "Lower priority numbers run first. Connections with the same number share traffic."
            : "Add the connections that should share traffic."}
        </p>
      </div>
      {members.length === 0 && (
        <div className="rounded-xl border border-dashed border-border p-4 text-[12px] text-muted-foreground">
          {!pool && <p>Add at least one connection to create a pool.</p>}
          {priority
            ? "Add a primary connection below, then a backup."
            : "Add the connections that should share traffic."}{" "}
          You can mix platform access and your own keys.
        </div>
      )}
      {selected.isError && (
        <ErrorBanner
          message="Could not refresh selected connections. Retry to check their compatibility."
          onRetry={() => {
            void selected.refetch();
          }}
        />
      )}
      {orderedMembers.map(({ member }, position) => {
        const id = member.user_service_id!;
        const candidate = selectedRows.get(id);
        const labelRow = candidate ?? selectedLabels[id] ?? savedRows.get(id);
        const requiresDeclaration =
          !aiChat &&
          priority &&
          (labelRow?.requires_compatibility_declaration ||
            member.same_api_compatible);
        return (
          <div
            key={id}
            className="space-y-3 rounded-xl border border-border/50 bg-card p-3"
          >
            <div className="flex items-start gap-3">
              <span className="flex size-6 shrink-0 items-center justify-center rounded-lg bg-white/[0.04] font-mono text-[11px] text-muted-foreground">
                {position + 1}
              </span>
              <div className="min-w-0 flex-1">
                <p className="break-words text-[12px] font-medium">
                  {labelRow?.name ||
                    labelRow?.slug ||
                    `Connection ${position + 1}`}
                </p>
                <p className="break-words text-[11px] text-muted-foreground">
                  {labelRow
                    ? `${labelRow.slug} · ${bindingLabel(labelRow.credential_binding)}`
                    : "Loading connection details…"}
                </p>
                {candidate?.reason && (
                  <p className="mt-1 text-[11px] text-warning">
                    {reason(candidate)}
                  </p>
                )}
              </div>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                aria-label={`Remove ${labelRow?.name || labelRow?.slug || `connection ${position + 1}`}`}
                onClick={() =>
                  form.setValue(
                    "members",
                    form
                      .getValues("members")
                      .filter((m) => m.user_service_id !== id),
                  )
                }
              >
                Remove
              </Button>
            </div>
            <div
              className={`grid gap-3 ${aiChat ? "sm:grid-cols-[100px_1fr]" : "sm:grid-cols-2"}`}
            >
              {priority && (
                <NumberInput
                  label={`Priority for member ${position + 1}`}
                  value={member.priority ?? 0}
                  max={4294967295}
                  onChange={(v) => setMember(id, { priority: v })}
                />
              )}
              {aiChat && (
                <label className="space-y-1 text-[12px]">
                  <span>Model (required)</span>
                  <Input
                    aria-label={`Model for member ${position + 1}`}
                    placeholder="Provider model name"
                    value={member.model ?? ""}
                    onChange={(e) =>
                      setMember(id, { model: e.target.value || null })
                    }
                  />
                </label>
              )}
              {weighted && (
                <NumberInput
                  label={`Weight for member ${position + 1}`}
                  min={1}
                  max={1000}
                  value={member.weight ?? 1}
                  onChange={(v) => setMember(id, { weight: v })}
                />
              )}
            </div>
            <Toggle
              label={`Member ${position + 1} enabled`}
              checked={member.enabled ?? true}
              onChange={(v) => setMember(id, { enabled: v })}
            />
            {requiresDeclaration && (
              <label className="flex items-start gap-2 text-[12px] leading-relaxed">
                <Checkbox
                  className="mt-0.5"
                  aria-label={`Confirm API compatibility for member ${position + 1}`}
                  checked={member.same_api_compatible ?? false}
                  onCheckedChange={(v) =>
                    setMember(id, { same_api_compatible: v === true })
                  }
                />
                I confirm this connection accepts the same paths and request
                format as the other connections.
              </label>
            )}
          </div>
        );
      })}
      <div className="space-y-2 rounded-xl border border-border/50 p-3">
        <h4 className="text-[12px] font-medium">Select connections</h4>
        <PoolConnectionPicker
          rows={rows}
          selectedIds={members.map((member) => member.user_service_id!)}
          search={search}
          onSearch={setSearch}
          onToggle={toggleMember}
          isLoading={candidates.isLoading}
          isSearching={
            search !== settledSearch ||
            (candidates.isFetching && !candidates.isFetchingNextPage)
          }
          isError={candidates.isError}
          error={candidates.error}
          onRetry={() => {
            void candidates.refetch();
          }}
          hasNextPage={candidates.hasNextPage}
          isFetchingNextPage={candidates.isFetchingNextPage}
          onLoadMore={() => {
            void candidates.fetchNextPage();
          }}
        />
      </div>
      {!aiChat && (
        <details className="rounded-xl border border-border/50 p-3">
          <summary className="cursor-pointer text-[12px] font-medium">
            Check an operation (optional)
          </summary>
          <div className="pt-3">
            <PoolOperationCheck operation={operation} onChange={setOperation} />
          </div>
        </details>
      )}
    </section>
  );
}
