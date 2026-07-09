import { useEffect, useMemo, useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { useWatch } from "react-hook-form";
import { firstNestedErrorMessage } from "@/lib/form-errors";
import { toast } from "sonner";
import {
  CheckCircle2,
  Edit3,
  MoreVertical,
  Trash2,
} from "lucide-react";
import { ApiError } from "@/lib/api-client";
import { formatRelativeTime } from "@/lib/utils";
import { AddCtaButton } from "@/components/shared/add-cta-button";
import { ErrorBanner } from "@/components/shared/error-banner";
import { HierarchyIcon } from "@/components/icons/empty-state";
import { Badge } from "@/components/ui/badge";
import { Button, ButtonIcon } from "@/components/ui/button";
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
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
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
  useServicePools,
  useSetServicePoolMembers,
  useUpdateServicePool,
} from "@/hooks/use-pools";
import { useUserServices } from "@/hooks/use-user-services";
import {
  createServicePoolSchema,
  type CreateServicePoolInput,
  type PoolStrategy,
  type ServicePool,
  type ServicePoolMember,
} from "@/schemas/pools";
import type { UserServiceResponse } from "@/schemas/keys";

interface ServicePoolsTabProps {
  readonly createOpen: boolean;
  readonly onCreateOpenChange: (open: boolean) => void;
}

type PoolFormValues = CreateServicePoolInput;

function strategyLabel(strategy: PoolStrategy): string {
  return strategy === "weighted" ? "Weighted" : "Round Robin";
}

function serviceLabel(service: UserServiceResponse): string {
  return service.slug;
}

function memberSummary(pool: ServicePool): string {
  const enabled = pool.members.filter((member) => member.enabled).length;
  return `${String(pool.members.length)} member${pool.members.length === 1 ? "" : "s"} / ${String(enabled)} enabled`;
}

function descriptionValue(value: string | null | undefined): string {
  return value?.trim() ?? "";
}

function toMemberMap(members: readonly ServicePoolMember[]) {
  return new Map(members.map((member) => [member.user_service_id, member]));
}

function normalizeDescription(value: string | undefined): string | undefined {
  const trimmed = value?.trim() ?? "";
  return trimmed ? trimmed : undefined;
}

function selectableServices(
  services: readonly UserServiceResponse[] | undefined,
): readonly UserServiceResponse[] {
  return services ?? [];
}

function CreatePoolDialog({
  open,
  onOpenChange,
  services,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly services: readonly UserServiceResponse[];
}) {
  const createMutation = useCreateServicePool();
  const form = useAppForm<PoolFormValues>({
    resolver: zodResolver(createServicePoolSchema),
    mode: "onChange",
    defaultValues: {
      slug: "",
      name: "",
      description: "",
      strategy: "round_robin",
      members: [],
      is_active: true,
    },
  });
  const selectedMembers =
    useWatch({ control: form.control, name: "members" }) ?? [];
  const selectedIds = new Set(selectedMembers.map((member) => member.user_service_id));

  useEffect(() => {
    if (!open) {
      form.reset({
        slug: "",
        name: "",
        description: "",
        strategy: "round_robin",
        members: [],
        is_active: true,
      });
    }
  }, [form, open]);

  function toggleMember(serviceId: string, checked: boolean) {
    const current = form.getValues("members");
    const next = checked
      ? [...current, { user_service_id: serviceId, weight: 1, enabled: true }]
      : current.filter((member) => member.user_service_id !== serviceId);
    form.setValue("members", next);
  }

  async function onSubmit(values: PoolFormValues) {
    try {
      await createMutation.mutateAsync({
        ...values,
        slug: values.slug.trim(),
        name: values.name.trim(),
        description: normalizeDescription(values.description),
        is_active: values.is_active ?? true,
      });
      toast.success("Service pool created");
      onOpenChange(false);
    } catch (error) {
      if (error instanceof ApiError) {
        form.setError("root", { message: error.message });
      } else {
        toast.error("Failed to create service pool");
      }
    }
  }

  const canSubmit =
    form.formState.isValid &&
    form.formState.isDirty &&
    !createMutation.isPending;
  // Member errors live at nested paths (members.{i}.weight) with no
  // FormField render site; surface the first one or the isValid gate
  // disables Save with zero feedback.
  const membersError = firstNestedErrorMessage(form.formState.errors.members);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="md:max-w-2xl">
        <DialogHeader>
          <DialogTitle>Create Service Pool</DialogTitle>
          <DialogDescription>
            Group interchangeable AI services behind one proxy slug.
          </DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form onSubmit={form.handleSubmit(onSubmit)} className="space-y-4">
            {form.formState.errors.root && (
              <div className="rounded-lg bg-destructive/10 p-3 text-[12px] text-destructive">
                {form.formState.errors.root.message}
              </div>
            )}
            <div className="grid gap-4 sm:grid-cols-2">
              <FormField
                control={form.control}
                name="name"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Name</FormLabel>
                    <FormControl>
                      <Input placeholder="Primary LLM Pool" {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name="slug"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Proxy Slug</FormLabel>
                    <FormControl>
                      <Input placeholder="llm-pool" {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
            </div>
            <FormField
              control={form.control}
              name="description"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Description</FormLabel>
                  <FormControl>
                    <textarea
                      className="min-h-20 w-full rounded-lg border border-input bg-transparent px-3 py-2 text-[12px] text-foreground placeholder:text-text-tertiary focus:outline-none focus:border-white/[0.15]"
                      placeholder="Optional notes for this pool"
                      {...field}
                    />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
            <div className="grid gap-4 sm:grid-cols-2">
              <FormField
                control={form.control}
                name="strategy"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Strategy</FormLabel>
                    <Select value={field.value} onValueChange={field.onChange}>
                      <FormControl>
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                      </FormControl>
                      <SelectContent>
                        <SelectItem value="round_robin">Round Robin</SelectItem>
                        <SelectItem value="weighted">Weighted</SelectItem>
                      </SelectContent>
                    </Select>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name="is_active"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Active</FormLabel>
                    <div className="flex h-8 items-center gap-2 rounded-lg border border-border px-3">
                      <FormControl>
                        <Switch
                          checked={field.value ?? true}
                          onCheckedChange={field.onChange}
                        />
                      </FormControl>
                      <span className="text-[12px] text-muted-foreground">
                        Accept proxy traffic
                      </span>
                    </div>
                    <FormMessage />
                  </FormItem>
                )}
              />
            </div>
            <div className="space-y-2">
              <p className="text-[12px] font-medium text-foreground">
                Member Services
              </p>
              {services.length === 0 ? (
                <div className="rounded-lg bg-white/[0.03] px-4 py-3 text-[12px] text-muted-foreground">
                  Add a service first, then include it in a pool.
                </div>
              ) : (
                <div className="max-h-60 space-y-2 overflow-y-auto rounded-xl border border-border/50 p-2">
                  {services.map((service) => (
                    <label
                      key={service.id}
                      className="flex items-center gap-3 rounded-lg px-2 py-2 hover:bg-white/[0.03]"
                    >
                      <Checkbox
                        checked={selectedIds.has(service.id)}
                        onCheckedChange={(checked) =>
                          toggleMember(service.id, checked === true)
                        }
                      />
                      <span className="min-w-0 flex-1">
                        <span className="block truncate text-[12px] font-medium text-foreground">
                          {serviceLabel(service)}
                        </span>
                        <span className="block truncate font-mono text-[11px] text-text-tertiary">
                          /proxy/s/{service.slug}
                        </span>
                      </span>
                    </label>
                  ))}
                </div>
              )}
              {membersError && (
                <p className="text-xs text-destructive">{membersError}</p>
              )}
            </div>
            <DialogFooter>
              <Button
                type="button"
                variant="outline"
                onClick={() => onOpenChange(false)}
              >
                Cancel
              </Button>
              <Button
                variant="primary"
                type="submit"
                isLoading={createMutation.isPending}
                disabled={!canSubmit}
              >
                Create Pool
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  );
}

function PoolEditorDialog({
  pool,
  services,
  open,
  onOpenChange,
}: {
  readonly pool: ServicePool | null;
  readonly services: readonly UserServiceResponse[];
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
}) {
  const updateMutation = useUpdateServicePool();
  const setMembersMutation = useSetServicePoolMembers();
  const form = useAppForm<PoolFormValues>({
    resolver: zodResolver(createServicePoolSchema),
    mode: "onChange",
    defaultValues: {
      slug: "",
      name: "",
      description: "",
      strategy: "round_robin",
      members: [],
      is_active: true,
    },
  });
  const members = useWatch({ control: form.control, name: "members" }) ?? [];
  const memberById = toMemberMap(members);

  useEffect(() => {
    if (!pool || !open) return;
    form.reset({
      slug: pool.slug,
      name: pool.name,
      description: descriptionValue(pool.description),
      strategy: pool.strategy,
      members: pool.members.map((member) => ({ ...member })),
      is_active: pool.is_active,
    });
  }, [form, open, pool]);

  const serviceOptions = useMemo(() => {
    if (!pool) return services;
    const known = new Set(services.map((service) => service.id));
    const missing = pool.members
      .filter((member) => !known.has(member.user_service_id))
      .map((member) => ({
        id: member.user_service_id,
        slug: member.user_service_id.slice(0, 8),
        resource_uri: `https://unavailable.invalid/api/v1/proxy/s/${member.user_service_id.slice(0, 8)}`,
        endpoint_id: "",
        api_key_id: null,
        auth_method: "",
        auth_key_name: "",
        catalog_service_id: null,
        node_id: null,
        node_priority: 0,
        is_active: false,
        admin_only: false,
        identity_propagation_mode: "none",
        identity_include_user_id: false,
        identity_include_email: false,
        identity_include_name: false,
        identity_jwt_audience: null,
        forward_access_token: false,
        inject_delegation_token: false,
        delegation_token_scope: "",
        ws_frame_injections: [],
        created_at: "",
        updated_at: "",
        credential_source: { type: "personal" as const },
      }));
    return [...services, ...missing];
  }, [pool, services]);

  function setMember(serviceId: string, checked: boolean) {
    const current = form.getValues("members");
    const next = checked
      ? [...current, { user_service_id: serviceId, weight: 1, enabled: true }]
      : current.filter((member) => member.user_service_id !== serviceId);
    form.setValue("members", next);
  }

  function updateMember(
    serviceId: string,
    patch: Partial<Pick<ServicePoolMember, "weight" | "enabled">>,
  ) {
    const next = form.getValues("members").map((member) =>
      member.user_service_id === serviceId ? { ...member, ...patch } : member,
    );
    form.setValue("members", next);
  }

  async function onSubmit(values: PoolFormValues) {
    if (!pool) return;
    try {
      await updateMutation.mutateAsync({
        poolId: pool.id,
        slug: values.slug.trim(),
        name: values.name.trim(),
        description: normalizeDescription(values.description),
        strategy: values.strategy,
        is_active: values.is_active,
      });
      await setMembersMutation.mutateAsync({
        poolId: pool.id,
        members: values.members,
      });
      toast.success("Service pool updated");
      onOpenChange(false);
    } catch (error) {
      if (error instanceof ApiError) {
        form.setError("root", { message: error.message });
      } else {
        toast.error("Failed to update service pool");
      }
    }
  }

  const isPending = updateMutation.isPending || setMembersMutation.isPending;
  const canSubmit =
    form.formState.isValid &&
    form.formState.isDirty &&
    !isPending;
  const membersError = firstNestedErrorMessage(form.formState.errors.members);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="md:max-w-3xl">
        <DialogHeader>
          <DialogTitle>Edit Service Pool</DialogTitle>
          <DialogDescription>
            Change routing strategy and member weights for this pool.
          </DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form onSubmit={form.handleSubmit(onSubmit)} className="space-y-4">
            {form.formState.errors.root && (
              <div className="rounded-lg bg-destructive/10 p-3 text-[12px] text-destructive">
                {form.formState.errors.root.message}
              </div>
            )}
            <div className="grid gap-4 sm:grid-cols-2">
              <FormField
                control={form.control}
                name="name"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Name</FormLabel>
                    <FormControl>
                      <Input {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name="slug"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Proxy Slug</FormLabel>
                    <FormControl>
                      <Input {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
            </div>
            <FormField
              control={form.control}
              name="description"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Description</FormLabel>
                  <FormControl>
                    <textarea
                      className="min-h-20 w-full rounded-lg border border-input bg-transparent px-3 py-2 text-[12px] text-foreground placeholder:text-text-tertiary focus:outline-none focus:border-white/[0.15]"
                      {...field}
                    />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
            <div className="grid gap-4 sm:grid-cols-2">
              <FormField
                control={form.control}
                name="strategy"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Strategy</FormLabel>
                    <Select value={field.value} onValueChange={field.onChange}>
                      <FormControl>
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                      </FormControl>
                      <SelectContent>
                        <SelectItem value="round_robin">Round Robin</SelectItem>
                        <SelectItem value="weighted">Weighted</SelectItem>
                      </SelectContent>
                    </Select>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name="is_active"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Active</FormLabel>
                    <div className="flex h-8 items-center gap-2 rounded-lg border border-border px-3">
                      <FormControl>
                        <Switch
                          checked={field.value ?? true}
                          onCheckedChange={field.onChange}
                        />
                      </FormControl>
                      <span className="text-[12px] text-muted-foreground">
                        Accept proxy traffic
                      </span>
                    </div>
                    <FormMessage />
                  </FormItem>
                )}
              />
            </div>
            <div className="space-y-2">
              <p className="text-[12px] font-medium text-foreground">
                Member Services
              </p>
              <div className="max-h-[22rem] space-y-2 overflow-y-auto rounded-xl border border-border/50 p-2">
                {serviceOptions.map((service) => {
                  const member = memberById.get(service.id);
                  const checked = Boolean(member);
                  return (
                    <div
                      key={service.id}
                      className="grid gap-3 rounded-lg px-2 py-2 hover:bg-white/[0.03] sm:grid-cols-[minmax(0,1fr)_88px_72px]"
                    >
                      <label className="flex min-w-0 items-center gap-3">
                        <Checkbox
                          checked={checked}
                          onCheckedChange={(next) =>
                            setMember(service.id, next === true)
                          }
                        />
                        <span className="min-w-0 flex-1">
                          <span className="block truncate text-[12px] font-medium text-foreground">
                            {serviceLabel(service)}
                          </span>
                          <span className="block truncate font-mono text-[11px] text-text-tertiary">
                            {service.is_active ? `/proxy/s/${service.slug}` : "Unavailable"}
                          </span>
                        </span>
                      </label>
                      <Input
                        type="number"
                        min={1}
                        max={1000}
                        value={member?.weight ?? 1}
                        disabled={!checked}
                        onChange={(event) =>
                          updateMember(service.id, {
                            // Clamp to the schema range (int 1-1000) so typed
                            // values can't silently disable the isValid gate.
                            weight: Math.min(
                              1000,
                              Math.max(
                                1,
                                Math.round(Number(event.target.value) || 1),
                              ),
                            ),
                          })
                        }
                        aria-label={`Weight for ${service.slug}`}
                      />
                      <div className="flex items-center gap-2">
                        <Switch
                          checked={member?.enabled ?? true}
                          disabled={!checked}
                          onCheckedChange={(enabled) =>
                            updateMember(service.id, { enabled })
                          }
                          aria-label={`Enable ${service.slug}`}
                        />
                        <span className="text-[11px] text-muted-foreground">
                          Enabled
                        </span>
                      </div>
                    </div>
                  );
                })}
              </div>
              {membersError && (
                <p className="text-xs text-destructive">{membersError}</p>
              )}
            </div>
            <DialogFooter>
              <Button
                type="button"
                variant="outline"
                onClick={() => onOpenChange(false)}
              >
                Cancel
              </Button>
              <Button
                variant="primary"
                type="submit"
                isLoading={isPending}
                disabled={!canSubmit}
              >
                Save Changes
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  );
}

function PoolActions({
  pool,
  onEdit,
  onDelete,
}: {
  readonly pool: ServicePool;
  readonly onEdit: (pool: ServicePool) => void;
  readonly onDelete: (pool: ServicePool) => void;
}) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="ghost" size="icon" className="h-7 w-7">
          <MoreVertical className="h-3.5 w-3.5" aria-hidden="true" />
          <span className="sr-only">Actions for {pool.name}</span>
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuItem onClick={() => onEdit(pool)}>
          <Edit3 className="mr-2 h-4 w-4" aria-hidden="true" />
          Edit
        </DropdownMenuItem>
        <DropdownMenuItem
          onClick={() => onDelete(pool)}
          className="text-destructive focus:text-destructive"
        >
          <Trash2 className="mr-2 h-4 w-4 text-destructive" aria-hidden="true" />
          Delete
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function PoolActiveSwitch({ pool }: { readonly pool: ServicePool }) {
  const updateMutation = useUpdateServicePool();

  async function handleChange(isActive: boolean) {
    try {
      await updateMutation.mutateAsync({
        poolId: pool.id,
        is_active: isActive,
      });
      toast.success(isActive ? "Service pool activated" : "Service pool disabled");
    } catch (error) {
      toast.error(
        error instanceof ApiError
          ? error.message
          : "Failed to update service pool",
      );
    }
  }

  return (
    <div className="flex items-center gap-2">
      <Switch
        checked={pool.is_active}
        disabled={updateMutation.isPending}
        onCheckedChange={(checked) => void handleChange(checked)}
        aria-label={`Toggle ${pool.name}`}
      />
      <span className="text-[11px] text-muted-foreground">
        {pool.is_active ? "Active" : "Inactive"}
      </span>
    </div>
  );
}

function PoolMobileCard({
  pool,
  onEdit,
  onDelete,
}: {
  readonly pool: ServicePool;
  readonly onEdit: (pool: ServicePool) => void;
  readonly onDelete: (pool: ServicePool) => void;
}) {
  return (
    <div className="relative rounded-xl border border-border/50 bg-card p-4">
      <div className="absolute right-3 top-3">
        <PoolActions pool={pool} onEdit={onEdit} onDelete={onDelete} />
      </div>
      <p className="pr-10 text-[13px] font-semibold text-foreground truncate">
        {pool.name}
      </p>
      <p className="mt-0.5 truncate font-mono text-[11px] text-text-tertiary">
        /proxy/s/{pool.slug}
      </p>
      <div className="mt-2 flex flex-wrap gap-1.5">
        <Badge variant="secondary">{strategyLabel(pool.strategy)}</Badge>
      </div>
      <div className="mt-3">
        <PoolActiveSwitch pool={pool} />
      </div>
      <div className="mt-3 flex flex-wrap gap-x-4 gap-y-1 text-[11px] text-muted-foreground">
        <span>{memberSummary(pool)}</span>
        <span>Updated {formatRelativeTime(pool.updated_at)}</span>
      </div>
    </div>
  );
}

function PoolsEmptyState({ onAdd }: { readonly onAdd: () => void }) {
  return (
    <div className="flex flex-col items-center justify-center gap-1 py-12 text-center">
      <HierarchyIcon className="h-64 w-64 text-muted-foreground/30" />
      <div className="space-y-1">
        <p className="text-[12px] font-medium text-muted-foreground/30">
          No service pools yet
        </p>
        <p className="text-xs text-muted-foreground/30">
          Create a pool when multiple service credentials can serve the same job.
        </p>
      </div>
      <AddCtaButton label="Create Pool" onClick={onAdd} />
    </div>
  );
}

export function ServicePoolsTab({
  createOpen,
  onCreateOpenChange,
}: ServicePoolsTabProps) {
  const { data: pools, isLoading, error, refetch } = useServicePools();
  const { data: userServices } = useUserServices();
  const deleteMutation = useDeleteServicePool();
  const services = useMemo(() => selectableServices(userServices), [userServices]);
  const [editPool, setEditPool] = useState<ServicePool | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<ServicePool | null>(null);

  async function handleDelete() {
    if (!deleteTarget) return;
    try {
      await deleteMutation.mutateAsync(deleteTarget.id);
      toast.success("Service pool deleted");
      setDeleteTarget(null);
    } catch (error) {
      toast.error(
        error instanceof ApiError ? error.message : "Failed to delete service pool",
      );
    }
  }

  if (isLoading) {
    return (
      <div className="space-y-3">
        {Array.from({ length: 3 }).map((_, i) => (
          <Skeleton key={`pool-skel-${String(i)}`} className="h-16 w-full" />
        ))}
      </div>
    );
  }

  if (error) {
    return (
      <ErrorBanner
        message="Failed to load service pools. Please try again."
        onRetry={refetch}
      />
    );
  }

  const poolList = pools ?? [];

  return (
    <>
      {poolList.length === 0 ? (
        <PoolsEmptyState onAdd={() => onCreateOpenChange(true)} />
      ) : (
        <>
          <div className="flex flex-col gap-3 md:hidden">
            {poolList.map((pool) => (
              <PoolMobileCard
                key={pool.id}
                pool={pool}
                onEdit={setEditPool}
                onDelete={setDeleteTarget}
              />
            ))}
          </div>
          <div className="hidden md:block rounded-xl border border-border/50 bg-card overflow-hidden">
            <Table>
              <TableHeader>
                <TableRow className="border-border/50 hover:bg-transparent">
                  <TableHead className="w-[26%]">Name</TableHead>
                  <TableHead className="w-[22%]">Proxy Slug</TableHead>
                  <TableHead className="w-[14%]">Strategy</TableHead>
                  <TableHead className="w-[16%]">Members</TableHead>
                  <TableHead className="w-[12%]">Status</TableHead>
                  <TableHead className="w-[10%]">Actions</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {poolList.map((pool) => (
                  <TableRow key={pool.id} className="border-border/30">
                    <TableCell>
                      <p className="truncate font-medium text-foreground">
                        {pool.name}
                      </p>
                      <p className="truncate text-[11px] text-text-tertiary mt-0.5">
                        {pool.description ?? "No description"}
                      </p>
                    </TableCell>
                    <TableCell>
                      <code className="font-mono text-[11px] text-muted-foreground">
                        /proxy/s/{pool.slug}
                      </code>
                    </TableCell>
                    <TableCell>
                      <Badge variant="secondary">{strategyLabel(pool.strategy)}</Badge>
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {memberSummary(pool)}
                    </TableCell>
                    <TableCell>
                      <div className="flex flex-wrap items-center gap-2">
                        <PoolActiveSwitch pool={pool} />
                        {pool.members.some((member) => member.enabled) && (
                          <CheckCircle2 className="h-3.5 w-3.5 text-success" />
                        )}
                      </div>
                    </TableCell>
                    <TableCell>
                      <PoolActions
                        pool={pool}
                        onEdit={setEditPool}
                        onDelete={setDeleteTarget}
                      />
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        </>
      )}

      <CreatePoolDialog
        open={createOpen}
        onOpenChange={onCreateOpenChange}
        services={services}
      />
      <PoolEditorDialog
        pool={editPool}
        open={editPool !== null}
        onOpenChange={(open) => {
          if (!open) setEditPool(null);
        }}
        services={services}
      />

      <Dialog
        open={deleteTarget !== null}
        onOpenChange={(open) => {
          if (!open) setDeleteTarget(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete Service Pool</DialogTitle>
            <DialogDescription>
              Delete &quot;{deleteTarget?.name ?? ""}&quot;? Requests to its pool
              slug will stop resolving through this pool.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteTarget(null)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => void handleDelete()}
              isLoading={deleteMutation.isPending}
            >
              <ButtonIcon variant="destructive">
                <Trash2 className="h-3 w-3" />
              </ButtonIcon>
              Delete
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
