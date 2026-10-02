import { useEffect, useState } from "react";
import { useWatch, type UseFormReturn } from "react-hook-form";
import { ArrowDown, ArrowUp } from "lucide-react";
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
import { Choice, NumberInput, Toggle } from "./pool-controls";
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
  function memberLabel(id: string, fallbackIndex: number) {
    const row = selectedRows.get(id) ?? selectedLabels[id] ?? savedRows.get(id);
    return row?.name || row?.slug || `Connection ${fallbackIndex + 1}`;
  }
  function validWeight(value: number | undefined) {
    return (
      value === undefined ||
      (Number.isFinite(value) &&
        Number.isInteger(value) &&
        value >= 1 &&
        value <= 1000)
    );
  }
  function validPriority(value: number | undefined) {
    return (
      value === undefined ||
      (Number.isFinite(value) &&
        Number.isInteger(value) &&
        value >= 0 &&
        value <= 4294967295)
    );
  }
  function configuredShare(member: Partial<ServicePoolMember>): string | null {
    const memberWeight = member.weight ?? 1;
    if (member.enabled === false || !validWeight(memberWeight)) return null;
    const memberPriority = member.priority ?? 0;
    if (priority && !validPriority(memberPriority)) return null;
    const shareMembers =
      priority && values.tier_balance === "weighted"
        ? members.filter(
            (candidate) =>
              candidate.enabled !== false &&
              (candidate.priority ?? 0) === memberPriority,
          )
        : members.filter((candidate) => candidate.enabled !== false);
    if (shareMembers.some((candidate) => !validWeight(candidate.weight)))
      return null;
    const total = shareMembers.reduce(
      (sum, candidate) => sum + (candidate.weight ?? 1),
      0,
    );
    if (!Number.isFinite(total) || total <= 0) return null;
    const share = (memberWeight / total) * 100;
    if (!Number.isFinite(share) || share < 0) return null;
    if (share > 0 && share < 0.1) return "<0.1%";
    return Number.isInteger(share) ? `${share}%` : `${share.toFixed(1)}%`;
  }
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
  function moveMember(id: string, direction: -1 | 1) {
    const current = form.getValues("members");
    const index = current.findIndex((member) => member.user_service_id === id);
    const target = index + direction;
    if (index < 0 || target < 0 || target >= current.length) return;
    const next: ServicePoolMember[] = [...current];
    [next[index], next[target]] = [next[target]!, next[index]!];
    form.setValue("members", next);
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
      {priority && (
        <div className="space-y-2 rounded-xl border border-border/50 p-3">
          <Choice
            label="Connections with the same priority"
            value={values.tier_balance ?? "round_robin"}
            options={[
              ["round_robin", "Take turns"],
              ["weighted", "Share by weight"],
            ]}
            onChange={(value) =>
              form.setValue("tier_balance", value as "round_robin" | "weighted")
            }
          />
          <p className="text-[11px] text-muted-foreground">
            Lower priority numbers are tried first. Ties use this balancing
            mode; unavailable members can still be skipped at request time.
          </p>
        </div>
      )}
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
      {members.length === 0 && (
        <div className="rounded-xl border border-dashed border-border p-4 text-[12px] text-muted-foreground">
          {!pool && <p>Add at least one connection to create a pool.</p>}
          {priority
            ? "Choose a primary connection, then a backup."
            : "Add the connections that should share traffic."}{" "}
          You can mix platform access and your own keys.
        </div>
      )}
      {(values.strategy === "round_robin" || values.strategy === "weighted") &&
        members.length > 0 && (
          <div className="rounded-xl border border-border/50 bg-muted/20 p-3 text-[11px] text-muted-foreground">
            <p className="font-medium text-foreground">
              {values.strategy === "weighted"
                ? "Repeating weighted cycle"
                : "Repeating round-robin cycle"}
            </p>
            <p className="mt-1">
              {values.strategy === "weighted"
                ? "The saved order is the repeating cycle and each weight controls its configured share. The current position is retained between requests; this does not promise a next or fallback connection."
                : "The saved order is the repeating cycle. The current position is retained between requests, and disabled or unavailable connections can be skipped dynamically."}
            </p>
            <p className="mt-1 break-words text-[11px] text-muted-foreground">
              Cycle order:{" "}
              {members
                .map((member, index) =>
                  memberLabel(member.user_service_id!, index),
                )
                .join(" → ")}
            </p>
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
        const share = configuredShare(member);
        const invalidPriority = priority && !validPriority(member.priority);
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
                {!priority && (
                  <p className="mt-1 text-[11px] text-muted-foreground">
                    Cycle position {position + 1} of {members.length}
                    {position === 0 ? " · first in cycle" : ""}
                    {position === members.length - 1 ? " · last in cycle" : ""}
                    {values.strategy === "weighted"
                      ? member.enabled === false
                        ? " · disabled · excluded from configured share"
                        : share
                          ? ` · configured share ${share}`
                          : " · configured share unavailable until enabled weights are valid"
                      : " · repeats in this order"}
                  </p>
                )}
                {priority && values.tier_balance === "weighted" && (
                  <p className="mt-1 text-[11px] text-muted-foreground">
                    {invalidPriority
                      ? "Priority tier needs a valid number · configured share unavailable"
                      : `Priority tier ${member.priority ?? 0} · ${
                          share
                            ? `configured tier share ${share}`
                            : member.enabled === false
                              ? "disabled · excluded from tier share"
                              : "configured share unavailable until enabled weights in this tier are valid"
                        }`}
                  </p>
                )}
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
              {!priority && (
                <div className="flex shrink-0 items-center gap-1">
                  <Button
                    type="button"
                    variant="outline"
                    size="icon"
                    className="size-7"
                    aria-label={`Move connection ${position + 1} earlier in the cycle`}
                    disabled={position === 0}
                    onClick={() => moveMember(id, -1)}
                  >
                    <ArrowUp className="size-3.5" aria-hidden="true" />
                  </Button>
                  <Button
                    type="button"
                    variant="outline"
                    size="icon"
                    className="size-7"
                    aria-label={`Move connection ${position + 1} later in the cycle`}
                    disabled={position === members.length - 1}
                    onClick={() => moveMember(id, 1)}
                  >
                    <ArrowDown className="size-3.5" aria-hidden="true" />
                  </Button>
                </div>
              )}
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
