import { useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { useWatch } from "react-hook-form";
import { ArrowDown, ArrowUp, GripVertical, MoreVertical } from "lucide-react";
import { ServicePoolCards } from "./service-pool-cards";
import { reorderPoolMembers } from "@/lib/service-pool-display";
import { toast } from "sonner";
import { ApiError } from "@/lib/api-client";
import { firstNestedErrorMessage } from "@/lib/form-errors";
import { AddCtaButton } from "@/components/shared/add-cta-button";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  useAppForm,
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  useCreateServicePool,
  useDeleteServicePool,
  usePoolCandidates,
  usePoolHealth,
  useResetPoolHealth,
  useServicePools,
  useUpdateServicePool,
} from "@/hooks/use-pools";
import { useOrgs } from "@/hooks/use-orgs";
import {
  createServicePoolSchema,
  defaultFailoverPolicy,
  retryTriggers,
  type CreateServicePoolInput,
  type FailoverPolicy,
  type PoolCandidate,
  type ServicePool,
  type ServicePoolMember,
} from "@/schemas/pools";

interface ServicePoolsTabProps {
  readonly layout?: "cards" | "table";
  readonly initialOrgId?: string;
  readonly initialPoolId?: string;
  readonly createOpen: boolean;
  readonly onCreateOpenChange: (open: boolean) => void;
}
const readOnlyPreview =
  import.meta.env.DEV && import.meta.env.VITE_ROUTING_PREVIEW === "1";
const strategyLabels = {
  priority: "Priority",
  round_robin: "Round Robin",
  weighted: "Weighted",
};
const reasonLabels: Record<string, string> = {
  unavailable: "Connection unavailable",
  inactive: "Service disabled",
  disabled: "Member disabled",
  cooldown: "Cooling down",
  incompatible_protocol: "Protocol is incompatible with this pool",
  compatibility_declaration_required:
    "Confirm compatibility for every affected member",
  inference_protocol_required:
    "Catalog inference metadata or a supported chat operation is required",
  operation_unsupported: "Operation is not permitted",
  node_upgrade_required: "Upgrade the node for HTTP cancellation support",
  node_offline: "Node is offline",
  unsupported_transport: "Transport is not supported",
};
function reason(candidate: PoolCandidate) {
  return candidate.reason
    ? (reasonLabels[candidate.reason] ?? candidate.reason.replaceAll("_", " "))
    : "Eligible";
}
function message(error: unknown) {
  return error instanceof ApiError || error instanceof Error
    ? error.message
    : "Unable to save pool";
}
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

function Choice({
  label,
  value,
  onChange,
  options,
  disabled,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
  options: readonly [string, string][];
  disabled?: boolean;
}) {
  return (
    <label className="space-y-1 text-[12px]">
      <span>{label}</span>
      <Select value={value} onValueChange={onChange} disabled={disabled}>
        <SelectTrigger aria-label={label}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {options.map(([id, text]) => (
            <SelectItem key={id} value={id}>
              {text}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </label>
  );
}
function NumberInput({
  label,
  value,
  onChange,
  min = 0,
  max,
}: {
  label: string;
  value: number;
  onChange: (value: number) => void;
  min?: number;
  max?: number;
}) {
  return (
    <label className="space-y-1 text-[12px]">
      <span>{label}</span>
      <Input
        aria-label={label}
        type="number"
        min={min}
        max={max}
        value={value}
        onChange={(e) => onChange(e.target.valueAsNumber)}
      />
    </label>
  );
}
function Toggle({
  label,
  checked,
  onChange,
}: {
  label: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <label className="flex items-center justify-between gap-3 text-[12px]">
      <span>{label}</span>
      <Switch aria-label={label} checked={checked} onCheckedChange={onChange} />
    </label>
  );
}
function PolicyEditor({
  policy,
  onChange,
}: {
  policy: FailoverPolicy;
  onChange: (policy: FailoverPolicy) => void;
}) {
  const set = <K extends keyof FailoverPolicy>(
    key: K,
    value: FailoverPolicy[K],
  ) => onChange({ ...policy, [key]: value });
  return (
    <div className="space-y-4 rounded-xl border border-border/50 p-4">
      <div className="grid gap-3 sm:grid-cols-2">
        <NumberInput
          label="Maximum attempts"
          min={1}
          max={5}
          value={policy.max_attempts}
          onChange={(v) => set("max_attempts", v)}
        />
        <NumberInput
          label="Replay body limit (bytes)"
          min={1}
          value={policy.max_replay_body_bytes}
          onChange={(v) => set("max_replay_body_bytes", v)}
        />
        <NumberInput
          label="Attempt timeout (ms)"
          min={1000}
          max={300000}
          value={policy.per_attempt_timeout_ms}
          onChange={(v) => set("per_attempt_timeout_ms", v)}
        />
        <NumberInput
          label="Overall deadline (ms)"
          min={1000}
          max={600000}
          value={policy.overall_deadline_ms}
          onChange={(v) => set("overall_deadline_ms", v)}
        />
        <NumberInput
          label="Base cooldown (ms)"
          min={1}
          value={policy.cooldown.base_ms}
          onChange={(v) => set("cooldown", { ...policy.cooldown, base_ms: v })}
        />
        <NumberInput
          label="Maximum cooldown (ms)"
          min={1}
          max={3600000}
          value={policy.cooldown.max_ms}
          onChange={(v) => set("cooldown", { ...policy.cooldown, max_ms: v })}
        />
        <NumberInput
          label="Failures before cooldown"
          min={1}
          value={policy.cooldown.failures_to_open}
          onChange={(v) =>
            set("cooldown", { ...policy.cooldown, failures_to_open: v })
          }
        />
      </div>
      <Toggle
        label="Honor provider Retry-After"
        checked={policy.cooldown.honor_retry_after}
        onChange={(v) =>
          set("cooldown", { ...policy.cooldown, honor_retry_after: v })
        }
      />
      <fieldset className="space-y-2">
        <legend className="text-[12px] font-medium">Retry causes</legend>
        <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
          {retryTriggers.map((trigger) => (
            <label
              key={trigger}
              className="flex items-center gap-2 text-[12px]"
            >
              <Checkbox
                checked={policy.retry_on.includes(trigger)}
                onCheckedChange={(v) =>
                  set(
                    "retry_on",
                    v === true
                      ? [...policy.retry_on, trigger]
                      : policy.retry_on.filter((t) => t !== trigger),
                  )
                }
              />
              {trigger.replaceAll("_", " ")}
            </label>
          ))}
        </div>
      </fieldset>
      <Toggle
        label="Allow replay after an ambiguous dispatch"
        checked={policy.retry_ambiguous_dispatch}
        onChange={(v) => set("retry_ambiguous_dispatch", v)}
      />
      <p className="text-[12px] text-muted-foreground">
        A timeout or 5xx may follow completed work. Enabling replay for POST can
        duplicate provider work and charges. A provider 429 rejection can fall
        back without this option. Set maximum attempts to 1 to disable fallback.
      </p>
    </div>
  );
}

export function PoolEditor({
  pool,
  orgId,
  onClose,
}: {
  pool?: ServicePool;
  orgId?: string;
  onClose: () => void;
}) {
  const create = useCreateServicePool();
  const update = useUpdateServicePool();
  const form = useAppForm<CreateServicePoolInput>({
    resolver: zodResolver(createServicePoolSchema),
    mode: "onChange",
    defaultValues: {
      slug: pool?.slug ?? "",
      name: pool?.name ?? "",
      description: pool?.description ?? "",
      strategy: pool?.strategy ?? "priority",
      tier_balance: pool?.tier_balance ?? "round_robin",
      member_contract: pool?.member_contract ?? "same_api",
      failover: pool?.failover ?? null,
      members: pool?.members ?? [],
      is_active: pool?.is_active ?? true,
    },
  });
  const values = useWatch({ control: form.control });
  const members = values.members ?? [];
  const priority = values.strategy === "priority";
  const [search, setSearch] = useState("");
  const [dragged, setDragged] = useState<string | null>(null);
  const [orderAnnouncement, setOrderAnnouncement] = useState("");
  function moveMember(from: string, to: string) {
    if (from === to || form.getValues("strategy") !== "priority") return;
    const next = reorderPoolMembers(
      { strategy: "priority", members: form.getValues("members") },
      from,
      to,
    );
    form.setValue("members", next);
    setOrderAnnouncement("Priority order updated. Save to apply this route.");
  }
  const [candidateMethod, setCandidateMethod] = useState("POST");
  const [candidatePath, setCandidatePath] = useState("/");
  const candidates = usePoolCandidates({
    poolId: pool?.id,
    orgId,
    contract: values.member_contract,
    search,
    strategy: values.strategy,
    method: values.member_contract === "ai_chat" ? "POST" : candidateMethod,
    path:
      values.member_contract === "ai_chat" ? "chat/completions" : candidatePath,
    declaredPeerIds: members
      .filter((m) => m.same_api_compatible)
      .map((m) => m.user_service_id!),
    peerIds: members.map((m) => m.user_service_id!).filter(Boolean),
  });
  const rows = candidates.data?.pages.flatMap((page) => page.candidates) ?? [];
  const selectedRows = new Map(rows.map((row) => [row.user_service_id, row]));
  const [selectedLabels, setSelectedLabels] = useState<Record<string, string>>(
    {},
  );
  // Health fetches saved IDs directly, independently of candidate pagination.
  const savedMembers = usePoolHealth({
    poolId: pool?.id,
    contract: pool?.member_contract,
  });
  const savedLabels = new Map(
    savedMembers.data?.candidates.map((row) => [
      row.user_service_id,
      row.slug,
    ]) ?? [],
  );
  function setMember(id: string, patch: Partial<ServicePoolMember>) {
    form.setValue(
      "members",
      form
        .getValues("members")
        .map((m) => (m.user_service_id === id ? { ...m, ...patch } : m)),
    );
  }
  function setStrategy(value: string) {
    const strategy = value as CreateServicePoolInput["strategy"];
    form.setValue("strategy", strategy);
    if (strategy !== "priority") {
      form.setValue("member_contract", "same_api");
      form.setValue("tier_balance", "round_robin");
      form.setValue("failover", null);
      form.setValue(
        "members",
        form.getValues("members").map((m) => ({
          ...m,
          priority: 0,
          model: null,
          same_api_compatible: false,
        })),
      );
    }
  }
  async function save(input: CreateServicePoolInput) {
    if (readOnlyPreview) return;
    try {
      const normalized = {
        ...input,
        members: input.members.map((m) => ({
          ...m,
          model: m.model?.trim() || null,
        })),
      };
      if (pool)
        await update.mutateAsync({
          ...normalized,
          poolId: pool.id,
          expected_revision: pool.config_revision ?? 0,
          description: input.description?.trim() || null,
        });
      else
        await create.mutateAsync({
          ...normalized,
          description: input.description?.trim() || undefined,
          org_id: orgId,
        });
      toast.success(pool ? "Service pool saved" : "Service pool created");
      onClose();
    } catch (error) {
      form.setError("root", { message: message(error) });
    }
  }
  const rootError =
    form.formState.errors.root?.message ??
    firstNestedErrorMessage(form.formState.errors);
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent className="max-h-[90dvh] overflow-y-auto md:max-w-3xl">
        <DialogHeader>
          <DialogTitle>
            {pool ? "Edit service pool" : "Create service pool"}
          </DialogTitle>
          <DialogDescription>
            One route across compatible connections. Lower priority numbers run
            first. Ownership is{" "}
            {orgId ? "the selected organization" : "personal"}.
          </DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form onSubmit={form.handleSubmit(save)} className="space-y-4">
            <div className="grid gap-3 sm:grid-cols-2">
              {(["name", "slug"] as const).map((name) => (
                <FormField
                  key={name}
                  control={form.control}
                  name={name}
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{name === "name" ? "Name" : "Slug"}</FormLabel>
                      <FormControl>
                        <Input {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              ))}
            </div>
            <FormField
              control={form.control}
              name="description"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Description</FormLabel>
                  <FormControl>
                    <Input {...field} value={field.value ?? ""} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
            <div className="grid gap-3 sm:grid-cols-3">
              <Choice
                label="Strategy"
                value={values.strategy ?? "priority"}
                options={Object.entries(strategyLabels)}
                onChange={setStrategy}
              />
              {priority && (
                <>
                  <Choice
                    label="Request contract"
                    value={values.member_contract ?? "same_api"}
                    options={[
                      ["same_api", "Same API"],
                      ["ai_chat", "AI chat"],
                    ]}
                    onChange={(v) => {
                      form.setValue(
                        "member_contract",
                        v as "same_api" | "ai_chat",
                      );
                      if (v === "same_api")
                        form.setValue(
                          "members",
                          form
                            .getValues("members")
                            .map((m) => ({ ...m, model: null })),
                        );
                      void form.trigger();
                    }}
                  />
                  <Choice
                    label="Balance within a tier"
                    value={values.tier_balance ?? "round_robin"}
                    options={[
                      ["round_robin", "Round Robin"],
                      ["weighted", "Weighted"],
                    ]}
                    onChange={(v) =>
                      form.setValue(
                        "tier_balance",
                        v as "round_robin" | "weighted",
                      )
                    }
                  />
                </>
              )}
            </div>
            <Toggle
              label="Pool enabled"
              checked={values.is_active ?? true}
              onChange={(v) => form.setValue("is_active", v)}
            />
            <section className="space-y-3">
              <h3 className="text-[13px] font-semibold">Members</h3>
              {priority && (
                <p className="text-xs text-muted-foreground">
                  Drag or use arrows to set a strict priority order. To rotate
                  within a tier, give those members the same priority number.
                </p>
              )}
              <span className="sr-only" aria-live="polite">
                {orderAnnouncement}
              </span>
              {members.map((member, index) => {
                const id = member.user_service_id!;
                const candidate = selectedRows.get(id);
                return (
                  <div
                    key={id}
                    className="space-y-3 rounded-xl border border-border/50 p-3"
                    onDragOver={(event) => {
                      if (priority && dragged) event.preventDefault();
                    }}
                    onDrop={(event) => {
                      event.preventDefault();
                      if (
                        dragged &&
                        event.dataTransfer.getData(
                          "application/x-nyxid-pool",
                        ) === (pool?.id ?? "new")
                      )
                        moveMember(dragged, id);
                      setDragged(null);
                    }}
                  >
                    <div className="flex items-center justify-between gap-3">
                      {priority && (
                        <div className="flex shrink-0 items-center gap-1">
                          <Button
                            type="button"
                            variant="ghost"
                            size="icon"
                            className="size-7 cursor-grab"
                            draggable
                            aria-label={`Drag member ${index + 1}`}
                            onDragStart={(event) => {
                              event.dataTransfer.setData(
                                "application/x-nyxid-pool",
                                pool?.id ?? "new",
                              );
                              event.dataTransfer.effectAllowed = "move";
                              setDragged(id);
                            }}
                            onDragEnd={() => setDragged(null)}
                            onKeyDown={(event) => {
                              if (
                                event.key !== "ArrowUp" &&
                                event.key !== "ArrowDown"
                              )
                                return;
                              event.preventDefault();
                              const target =
                                members[
                                  index + (event.key === "ArrowUp" ? -1 : 1)
                                ]?.user_service_id;
                              if (target) moveMember(id, target);
                            }}
                          >
                            <GripVertical className="size-3.5" />
                          </Button>
                          <Button
                            type="button"
                            variant="ghost"
                            size="icon"
                            className="size-7"
                            aria-label={`Move member ${index + 1} up`}
                            disabled={index === 0}
                            onClick={() =>
                              moveMember(
                                id,
                                members[index - 1]!.user_service_id!,
                              )
                            }
                          >
                            <ArrowUp className="size-3.5" />
                          </Button>
                          <Button
                            type="button"
                            variant="ghost"
                            size="icon"
                            className="size-7"
                            aria-label={`Move member ${index + 1} down`}
                            disabled={index === members.length - 1}
                            onClick={() =>
                              moveMember(
                                id,
                                members[index + 1]!.user_service_id!,
                              )
                            }
                          >
                            <ArrowDown className="size-3.5" />
                          </Button>
                        </div>
                      )}
                      <div className="min-w-0 flex-1">
                        <p className="text-[12px] font-medium">
                          {candidate?.slug ??
                            selectedLabels[id] ??
                            savedLabels.get(id) ??
                            id}
                        </p>
                        <p className="text-[11px] text-muted-foreground">
                          {candidate
                            ? `${candidate.credential_binding} · ${candidate.protocol ?? "Same API"} · ${reason(candidate)}`
                            : "Select the candidate operation to inspect availability"}
                        </p>
                      </div>
                      <Button
                        type="button"
                        variant="ghost"
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
                    <div className="grid gap-3 sm:grid-cols-3">
                      {priority && (
                        <NumberInput
                          label={`Priority for member ${index + 1}`}
                          value={member.priority ?? 0}
                          onChange={(priority) => setMember(id, { priority })}
                        />
                      )}
                      <NumberInput
                        label={`Weight for member ${index + 1}`}
                        min={1}
                        max={1000}
                        value={member.weight ?? 1}
                        onChange={(weight) => setMember(id, { weight })}
                      />
                      {priority && values.member_contract === "ai_chat" && (
                        <label className="space-y-1 text-[12px]">
                          <span>
                            Model{" "}
                            {values.member_contract === "ai_chat"
                              ? "(required)"
                              : "(optional)"}
                          </span>
                          <Input
                            aria-label={`Model for member ${index + 1}`}
                            value={member.model ?? ""}
                            onChange={(e) =>
                              setMember(id, { model: e.target.value || null })
                            }
                          />
                        </label>
                      )}
                    </div>
                    <Toggle
                      label={`Member ${index + 1} enabled`}
                      checked={member.enabled ?? true}
                      onChange={(enabled) => setMember(id, { enabled })}
                    />
                    {priority && values.member_contract !== "ai_chat" && (
                      <label className="flex items-start gap-2 text-[12px]">
                        <Checkbox
                          aria-label={`Confirm API compatibility for member ${index + 1}`}
                          checked={member.same_api_compatible ?? false}
                          onCheckedChange={(v) =>
                            setMember(id, { same_api_compatible: v === true })
                          }
                        />
                        I confirm this connection accepts the same operations
                        and wire format as the other members.
                      </label>
                    )}
                  </div>
                );
              })}
              {values.member_contract !== "ai_chat" && (
                <div className="grid grid-cols-[100px_1fr] gap-3">
                  <Choice
                    label="Candidate method"
                    value={candidateMethod}
                    onChange={setCandidateMethod}
                    options={[
                      "GET",
                      "POST",
                      "PUT",
                      "PATCH",
                      "DELETE",
                      "HEAD",
                    ].map((m) => [m, m])}
                  />
                  <label className="space-y-1 text-[12px]">
                    <span>Candidate operation path</span>
                    <Input
                      aria-label="Candidate operation path"
                      value={candidatePath}
                      onChange={(e) => setCandidatePath(e.target.value)}
                    />
                  </label>
                </div>
              )}
              <Input
                aria-label="Search candidate services"
                placeholder="Search connections to add"
                value={search}
                onChange={(e) => setSearch(e.target.value)}
              />
              {candidates.isError && (
                <ErrorBanner
                  message={message(candidates.error)}
                  onRetry={() => {
                    void candidates.refetch();
                  }}
                />
              )}
              {candidates.isLoading && <Skeleton className="h-12" />}
              <div className="max-h-48 space-y-1 overflow-y-auto">
                {rows
                  .filter(
                    (row) =>
                      !members.some(
                        (m) => m.user_service_id === row.user_service_id,
                      ),
                  )
                  .map((row) => (
                    <div
                      key={row.user_service_id}
                      className="flex items-center justify-between gap-3 rounded-lg p-2 hover:bg-white/[0.03]"
                    >
                      <div>
                        <p className="text-[12px]">{row.slug}</p>
                        <p className="text-[11px] text-muted-foreground">
                          {row.credential_binding} ·{" "}
                          {row.protocol ?? "Same API"} · {reason(row)}
                        </p>
                      </div>
                      <Button
                        type="button"
                        disabled={
                          !row.eligible &&
                          row.reason !== "compatibility_declaration_required"
                        }
                        onClick={() => {
                          setSelectedLabels((labels) => ({
                            ...labels,
                            [row.user_service_id]: row.slug,
                          }));
                          form.setValue("members", [
                            ...form.getValues("members"),
                            newMember(row.user_service_id),
                          ]);
                        }}
                      >
                        Add
                      </Button>
                    </div>
                  ))}
              </div>
              {candidates.hasNextPage && (
                <Button
                  type="button"
                  isLoading={candidates.isFetchingNextPage}
                  onClick={() => {
                    void candidates.fetchNextPage();
                  }}
                >
                  Load more candidates
                </Button>
              )}
            </section>
            {priority && (
              <section className="space-y-3">
                <Toggle
                  label="Customize failover policy"
                  checked={values.failover != null}
                  onChange={(v) =>
                    form.setValue(
                      "failover",
                      v ? structuredClone(defaultFailoverPolicy) : null,
                    )
                  }
                />
                {values.failover ? (
                  <PolicyEditor
                    policy={values.failover as FailoverPolicy}
                    onChange={(v) => form.setValue("failover", v)}
                  />
                ) : (
                  <p className="text-[12px] text-muted-foreground">
                    Defaults: up to 3 attempts, 60 seconds per attempt, 120
                    seconds overall, and 5–300 second cooldown. Ambiguous POST
                    replay is off.
                  </p>
                )}
              </section>
            )}
            {readOnlyPreview && (
              <p className="text-xs text-muted-foreground">
                Production preview: inspect or draft settings here. Saving pool
                changes is disabled.
              </p>
            )}
            {rootError && <ErrorBanner message={rootError} />}
            <DialogFooter>
              <Button type="button" variant="outline" onClick={onClose}>
                Cancel
              </Button>
              <Button
                type="submit"
                variant="primary"
                isLoading={create.isPending || update.isPending}
                disabled={
                  readOnlyPreview ||
                  !form.formState.isDirty ||
                  !form.formState.isValid
                }
              >
                Save
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  );
}

export function PoolHealthDialog({
  pool,
  onClose,
}: {
  pool: ServicePool;
  onClose: () => void;
}) {
  const [method, setMethod] = useState("POST");
  const [path, setPath] = useState(
    pool.member_contract === "ai_chat" ? "chat/completions" : "/",
  );
  const health = usePoolHealth({
    poolId: pool.id,
    method,
    path,
    contract: pool.member_contract,
  });
  const reset = useResetPoolHealth();
  async function clear(userServiceId?: string) {
    try {
      await reset.mutateAsync({ poolId: pool.id, userServiceId });
      toast.success("Cooldown reset");
    } catch (error) {
      toast.error(message(error));
    }
  }
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent className="md:max-w-2xl">
        <DialogHeader>
          <DialogTitle>{pool.name} health</DialogTitle>
          <DialogDescription>
            Health belongs to the effective credential, destination, model and
            operation. Reset permits the next normal attempt.
          </DialogDescription>
        </DialogHeader>
        <div className="grid grid-cols-[100px_1fr] gap-3">
          <Choice
            label="Method"
            value={method}
            options={["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"].map(
              (m) => [m, m],
            )}
            onChange={setMethod}
          />
          <label className="space-y-1 text-[12px]">
            <span>Operation path</span>
            <Input
              aria-label="Operation path"
              value={path}
              onChange={(e) => setPath(e.target.value)}
            />
          </label>
        </div>
        {health.isError && <ErrorBanner message={message(health.error)} />}
        {health.isLoading && <Skeleton className="h-16" />}
        <div className="space-y-2">
          {health.data?.candidates.map((row) => (
            <div
              key={row.user_service_id}
              className="flex items-center justify-between gap-3 rounded-xl border border-border/50 p-3"
            >
              <div>
                <p className="text-[12px] font-medium">
                  {row.slug}{" "}
                  <Badge variant={row.eligible ? "success" : "warning"}>
                    {reason(row)}
                  </Badge>
                </p>
                <p className="text-[11px] text-muted-foreground">
                  {row.credential_binding} · {row.consecutive_failures} failures
                  {row.last_status ? ` · HTTP ${row.last_status}` : ""}
                  {row.cooldown_until
                    ? ` · Retry after ${new Date(row.cooldown_until).toLocaleString()}`
                    : ""}
                </p>
              </div>
              <Button
                onClick={() => {
                  void clear(row.user_service_id);
                }}
                disabled={readOnlyPreview || reset.isPending}
              >
                Reset
              </Button>
            </div>
          ))}
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Close
          </Button>
          <Button
            onClick={() => {
              void clear();
            }}
            isLoading={reset.isPending}
            disabled={readOnlyPreview}
          >
            Reset all cooldowns
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function ServicePoolsTab({
  layout = "table",
  initialOrgId,
  initialPoolId,
  createOpen,
  onCreateOpenChange,
}: ServicePoolsTabProps) {
  const [owner, setOwner] = useState(initialOrgId ?? "personal");
  const orgId = owner === "personal" ? undefined : owner;
  const { data: orgs } = useOrgs();
  const pools = useServicePools(orgId);
  const update = useUpdateServicePool();
  const remove = useDeleteServicePool();
  const [editing, setEditing] = useState<ServicePool | null>(null);
  const [health, setHealth] = useState<ServicePool | null>(null);
  const [deleting, setDeleting] = useState<ServicePool | null>(null);
  async function toggle(pool: ServicePool) {
    try {
      await update.mutateAsync({
        poolId: pool.id,
        expected_revision: pool.config_revision ?? 0,
        is_active: !pool.is_active,
      });
      toast.success(pool.is_active ? "Pool disabled" : "Pool enabled");
    } catch (error) {
      toast.error(message(error));
    }
  }
  const actions = (pool: ServicePool) => (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          aria-label={`Actions for ${pool.name}`}
        >
          <MoreVertical className="size-4" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuItem onSelect={() => setEditing(pool)}>
          Edit
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={() => setHealth(pool)}>
          Health
        </DropdownMenuItem>
        <DropdownMenuItem
          disabled={readOnlyPreview || update.isPending}
          onSelect={() => {
            void toggle(pool);
          }}
        >
          {pool.is_active ? "Disable" : "Enable"}
        </DropdownMenuItem>
        <DropdownMenuItem
          className="text-destructive"
          disabled={readOnlyPreview}
          onSelect={() => setDeleting(pool)}
        >
          Delete
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div className="min-w-52">
          <Choice
            label="Pool owner"
            value={owner}
            onChange={setOwner}
            options={[
              ["personal", "Personal"],
              ...(orgs ?? [])
                .filter((o) => ["owner", "admin"].includes(o.your_role))
                .map(
                  (o) => [o.id, o.display_name ?? o.slug] as [string, string],
                ),
            ]}
          />
        </div>
        <AddCtaButton
          label="Create pool"
          onClick={() => onCreateOpenChange(true)}
        />
      </div>
      <p className="text-[12px] text-muted-foreground">
        Route through <code>/api/v1/proxy/s/&lt;slug&gt;</code>. AI chat pools
        also accept <code>model: pool:&lt;slug&gt;</code> at the LLM gateway.
      </p>
      {pools.isError && (
        <ErrorBanner
          message={message(pools.error)}
          onRetry={() => {
            void pools.refetch();
          }}
        />
      )}
      {pools.isLoading && <Skeleton className="h-24" />}
      {pools.data?.length === 0 && (
        <div className="rounded-xl border border-border/50 bg-card p-6 text-center text-[12px] text-muted-foreground">
          No service pools for this owner. Create a pool to group compatible
          connections.
        </div>
      )}
      {(pools.data?.length ?? 0) > 0 &&
        (layout === "cards" ? (
          <ServicePoolCards
            key={owner}
            pools={pools.data ?? []}
            initialOpenId={orgId === initialOrgId ? initialPoolId : undefined}
            actions={actions}
            onEdit={setEditing}
          />
        ) : (
          <>
            <div className="hidden overflow-hidden rounded-xl border border-border/50 bg-card md:block">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Name</TableHead>
                    <TableHead>Route</TableHead>
                    <TableHead>Strategy / contract</TableHead>
                    <TableHead>Members</TableHead>
                    <TableHead>Status</TableHead>
                    <TableHead>Actions</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {pools.data?.map((pool) => (
                    <TableRow key={pool.id}>
                      <TableCell>{pool.name}</TableCell>
                      <TableCell className="font-mono">{pool.slug}</TableCell>
                      <TableCell>
                        {strategyLabels[pool.strategy]} /{" "}
                        {pool.member_contract === "ai_chat"
                          ? "AI chat"
                          : "Same API"}
                      </TableCell>
                      <TableCell>
                        {pool.members.filter((m) => m.enabled).length} /{" "}
                        {pool.members.length} enabled
                      </TableCell>
                      <TableCell>
                        <Badge
                          variant={pool.is_active ? "success" : "secondary"}
                        >
                          {pool.is_active ? "Enabled" : "Disabled"}
                        </Badge>
                      </TableCell>
                      <TableCell>{actions(pool)}</TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>
            <div className="space-y-3 md:hidden">
              {pools.data?.map((pool) => (
                <div
                  key={pool.id}
                  className="flex items-center justify-between rounded-xl border border-border/50 bg-card p-4"
                >
                  <div>
                    <p className="text-[13px] font-medium">{pool.name}</p>
                    <p className="text-[12px] text-muted-foreground">
                      {pool.slug} · {strategyLabels[pool.strategy]} ·{" "}
                      {pool.is_active ? "Enabled" : "Disabled"}
                    </p>
                  </div>
                  {actions(pool)}
                </div>
              ))}
            </div>
          </>
        ))}
      {createOpen && (
        <PoolEditor orgId={orgId} onClose={() => onCreateOpenChange(false)} />
      )}
      {editing && (
        <PoolEditor
          key={editing.id}
          pool={editing}
          orgId={orgId}
          onClose={() => setEditing(null)}
        />
      )}
      {health && (
        <PoolHealthDialog pool={health} onClose={() => setHealth(null)} />
      )}
      {deleting && (
        <Dialog
          open
          onOpenChange={(open) => {
            if (!open) setDeleting(null);
          }}
        >
          <DialogContent>
            <DialogHeader>
              <DialogTitle>Delete {deleting.name}?</DialogTitle>
              <DialogDescription>
                The pool route will stop working. Member connections stay
                available.
              </DialogDescription>
            </DialogHeader>
            <DialogFooter>
              <Button variant="outline" onClick={() => setDeleting(null)}>
                Cancel
              </Button>
              <Button
                variant="destructive"
                isLoading={remove.isPending}
                onClick={() => {
                  void remove
                    .mutateAsync(deleting.id)
                    .then(() => {
                      setDeleting(null);
                      toast.success("Pool deleted");
                    })
                    .catch((error) => toast.error(message(error)));
                }}
              >
                Delete
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      )}
    </div>
  );
}
