import { useDeferredValue, useMemo, useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { Pencil, Plus, Search, Trash2 } from "lucide-react";
import type { UseFormReturn } from "react-hook-form";
import { toast } from "sonner";
import { useAdminUsers } from "@/hooks/use-admin";
import {
  useAdminAllowances,
  useAdminCreditGrants,
  useCreateAllowance,
  useIssueCreditGrant,
  useRevokeCreditGrant,
  useUpdateAllowance,
} from "@/hooks/use-billing-credits";
import { useServices } from "@/hooks/use-services";
import { ApiError } from "@/lib/api-client";
import {
  allowanceFormSchema,
  issueGrantFormSchema,
  type AllowanceForm,
  type CreditGrant,
  type IssueGrantForm,
  type UsageAllowance,
} from "@/schemas/billing-credits";
import { useAuthStore } from "@/stores/auth-store";
import type { AdminUser } from "@/types/admin";
import type { DownstreamService } from "@/types/api";
import { canAdminWrite } from "@/types/api";
import { PageHeader } from "@/components/shared/page-header";
import { ErrorBanner } from "@/components/shared/error-banner";
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
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  useAppForm,
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
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";

const GRANT_DEFAULTS: IssueGrantForm = {
  amount_credits: 100,
  target_kind: "all_users",
  target_user_ids: [],
  all_services: true,
  service_refs: [],
  expires_at: "",
  reason: "",
};

const ALLOWANCE_DEFAULTS: AllowanceForm = {
  service_ref: "",
  quantity: 1_000,
  recurrence: "monthly",
  target_kind: "all_users",
  target_user_ids: [],
};

export function AdminCreditsPage() {
  const currentUser = useAuthStore((state) => state.user);
  const canWrite = canAdminWrite(currentUser);
  const grantsQuery = useAdminCreditGrants();
  const allowancesQuery = useAdminAllowances();
  const servicesQuery = useServices();
  const issueGrant = useIssueCreditGrant();
  const revokeGrant = useRevokeCreditGrant();
  const createAllowance = useCreateAllowance();
  const updateAllowance = useUpdateAllowance();
  const [grantOpen, setGrantOpen] = useState(false);
  const [allowanceOpen, setAllowanceOpen] = useState(false);
  const [editingAllowance, setEditingAllowance] =
    useState<UsageAllowance | null>(null);
  const [grantToRevoke, setGrantToRevoke] = useState<CreditGrant | null>(null);

  const grantForm = useAppForm<IssueGrantForm>({
    resolver: zodResolver(issueGrantFormSchema),
    defaultValues: GRANT_DEFAULTS,
  });
  const allowanceForm = useAppForm<AllowanceForm>({
    resolver: zodResolver(allowanceFormSchema),
    defaultValues: ALLOWANCE_DEFAULTS,
  });

  function openGrantDialog() {
    grantForm.reset(GRANT_DEFAULTS);
    setGrantOpen(true);
  }

  function openAllowanceDialog(allowance?: UsageAllowance) {
    setEditingAllowance(allowance ?? null);
    allowanceForm.reset(
      allowance
        ? {
            service_ref: allowance.service_id,
            quantity: allowance.quantity,
            recurrence: allowance.recurrence,
            target_kind: allowance.target_kind,
            target_user_ids: allowance.target_user_ids,
          }
        : ALLOWANCE_DEFAULTS,
    );
    setAllowanceOpen(true);
  }

  async function submitGrant(value: IssueGrantForm) {
    try {
      const result = await issueGrant.mutateAsync(value);
      const recipientLabel = `${String(result.created_count)} owner${result.created_count === 1 ? "" : "s"}`;
      toast.success(
        result.pending_activation_count > 0
          ? `Issued credits to ${recipientLabel}; ${String(result.pending_activation_count)} pending activation`
          : `Issued credits to ${recipientLabel}`,
      );
      setGrantOpen(false);
    } catch (error) {
      toast.error(errorMessage(error, "Failed to issue credits"));
    }
  }

  async function submitAllowance(value: AllowanceForm) {
    try {
      const normalized = {
        ...value,
        target_user_ids:
          value.target_kind === "all_users" ? [] : value.target_user_ids,
      };
      if (editingAllowance) {
        await updateAllowance.mutateAsync({
          id: editingAllowance.id,
          body: normalized,
        });
        toast.success("Allowance updated");
      } else {
        await createAllowance.mutateAsync(normalized);
        toast.success("Allowance created");
      }
      setAllowanceOpen(false);
    } catch (error) {
      toast.error(errorMessage(error, "Failed to save allowance"));
    }
  }

  async function handleRevoke() {
    if (!grantToRevoke) return;
    try {
      await revokeGrant.mutateAsync(grantToRevoke.id);
      toast.success("Grant revoked");
      setGrantToRevoke(null);
    } catch (error) {
      toast.error(errorMessage(error, "Failed to revoke grant"));
    }
  }

  async function toggleAllowance(allowance: UsageAllowance) {
    try {
      await updateAllowance.mutateAsync({
        id: allowance.id,
        body: { is_active: !allowance.is_active },
      });
      toast.success(
        allowance.is_active ? "Allowance disabled" : "Allowance enabled",
      );
    } catch (error) {
      toast.error(errorMessage(error, "Failed to update allowance"));
    }
  }

  const services = servicesQuery.data ?? [];

  return (
    <div className="space-y-6">
      <PageHeader
        title="Credits"
        description="Manage promotional credit grants and recurring free usage allowances."
      />

      <Tabs defaultValue="grants">
        <TabsList>
          <TabsTrigger value="grants">Credit grants</TabsTrigger>
          <TabsTrigger value="allowances">Free allowances</TabsTrigger>
        </TabsList>

        <TabsContent value="grants" className="space-y-4">
          <div className="flex items-center justify-between gap-3">
            <p className="text-[12px] text-muted-foreground">
              Promotional credits are consumed before purchased wallet credits.
            </p>
            {canWrite ? (
              <Button size="sm" variant="primary" onClick={openGrantDialog}>
                <ButtonIcon variant="primary">
                  <Plus className="h-3.5 w-3.5" />
                </ButtonIcon>
                Issue grant
              </Button>
            ) : null}
          </div>
          {grantsQuery.isError ? (
            <ErrorBanner
              message={errorMessage(grantsQuery.error, "Failed to load grants")}
              onRetry={() => void grantsQuery.refetch()}
            />
          ) : grantsQuery.isLoading ? (
            <Skeleton className="h-52 w-full" />
          ) : (
            <div className="overflow-x-auto rounded-lg border border-border">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Recipient</TableHead>
                    <TableHead>Remaining</TableHead>
                    <TableHead>Scope</TableHead>
                    <TableHead>Expiry</TableHead>
                    <TableHead>Status</TableHead>
                    <TableHead>Reason</TableHead>
                    {canWrite ? (
                      <TableHead className="text-right">Actions</TableHead>
                    ) : null}
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {(grantsQuery.data?.grants ?? []).map((grant) => (
                    <TableRow key={grant.id}>
                      <TableCell>
                        <div className="font-medium">
                          {grant.recipient_display_name ||
                            grant.recipient_email ||
                            grant.recipient_user_id}
                        </div>
                        {grant.recipient_display_name &&
                        grant.recipient_email ? (
                          <div className="text-[11px] text-muted-foreground">
                            {grant.recipient_email}
                          </div>
                        ) : null}
                      </TableCell>
                      <TableCell>
                        {formatCredits(grant.remaining_micros)}{" "}
                        <span className="text-[11px] text-muted-foreground">
                          of {formatCredits(grant.amount_micros)}
                        </span>
                      </TableCell>
                      <TableCell>
                        {scopeLabel(
                          grant.scope.all_services,
                          grant.scope.service_slugs,
                        )}
                      </TableCell>
                      <TableCell>{formatDateTime(grant.expires_at)}</TableCell>
                      <TableCell>
                        <StatusBadge status={grant.status} />
                      </TableCell>
                      <TableCell className="max-w-56 truncate text-muted-foreground">
                        {grant.reason || "-"}
                      </TableCell>
                      {canWrite ? (
                        <TableCell className="text-right">
                          <Button
                            type="button"
                            size="icon"
                            variant="ghost"
                            title="Revoke grant"
                            disabled={
                              grant.status !== "active" ||
                              grant.reserved_micros > 0 ||
                              revokeGrant.isPending
                            }
                            onClick={() => setGrantToRevoke(grant)}
                          >
                            <Trash2 className="h-4 w-4" />
                          </Button>
                        </TableCell>
                      ) : null}
                    </TableRow>
                  ))}
                  {(grantsQuery.data?.grants.length ?? 0) === 0 ? (
                    <TableRow>
                      <TableCell
                        colSpan={canWrite ? 7 : 6}
                        className="py-10 text-center text-muted-foreground"
                      >
                        No credit grants.
                      </TableCell>
                    </TableRow>
                  ) : null}
                </TableBody>
              </Table>
            </div>
          )}
        </TabsContent>

        <TabsContent value="allowances" className="space-y-4">
          <div className="flex items-center justify-between gap-3">
            <p className="text-[12px] text-muted-foreground">
              Free metric units reset on UTC recurrence windows and settle
              against actual usage.
            </p>
            {canWrite ? (
              <Button
                size="sm"
                variant="primary"
                onClick={() => openAllowanceDialog()}
              >
                <ButtonIcon variant="primary">
                  <Plus className="h-3.5 w-3.5" />
                </ButtonIcon>
                Create allowance
              </Button>
            ) : null}
          </div>
          {allowancesQuery.isError ? (
            <ErrorBanner
              message={errorMessage(
                allowancesQuery.error,
                "Failed to load allowances",
              )}
              onRetry={() => void allowancesQuery.refetch()}
            />
          ) : allowancesQuery.isLoading ? (
            <Skeleton className="h-52 w-full" />
          ) : (
            <div className="overflow-x-auto rounded-lg border border-border">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Service</TableHead>
                    <TableHead>Quantity</TableHead>
                    <TableHead>Recurrence</TableHead>
                    <TableHead>Targets</TableHead>
                    <TableHead>Status</TableHead>
                    {canWrite ? (
                      <TableHead className="text-right">Actions</TableHead>
                    ) : null}
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {(allowancesQuery.data?.allowances ?? []).map((allowance) => (
                    <TableRow key={allowance.id}>
                      <TableCell>
                        <div className="font-medium">
                          {serviceName(
                            services,
                            allowance.service_id,
                            allowance.service_slug,
                          )}
                        </div>
                        <div className="text-[11px] text-muted-foreground">
                          {allowance.metric}
                        </div>
                      </TableCell>
                      <TableCell>{formatNumber(allowance.quantity)}</TableCell>
                      <TableCell className="capitalize">
                        {allowance.recurrence.replace("_", " ")}
                      </TableCell>
                      <TableCell>
                        {allowance.target_kind === "all_users"
                          ? "All owners"
                          : `${String(allowance.target_user_ids.length)} selected`}
                      </TableCell>
                      <TableCell>
                        <Badge
                          variant={
                            allowance.is_active ? "success" : "secondary"
                          }
                        >
                          {allowance.is_active ? "Active" : "Disabled"}
                        </Badge>
                      </TableCell>
                      {canWrite ? (
                        <TableCell className="text-right">
                          <div className="flex justify-end gap-1">
                            <Button
                              type="button"
                              size="icon"
                              variant="ghost"
                              title="Edit allowance"
                              onClick={() => openAllowanceDialog(allowance)}
                            >
                              <Pencil className="h-4 w-4" />
                            </Button>
                            <Button
                              type="button"
                              size="sm"
                              variant="ghost"
                              disabled={updateAllowance.isPending}
                              onClick={() => void toggleAllowance(allowance)}
                            >
                              {allowance.is_active ? "Disable" : "Enable"}
                            </Button>
                          </div>
                        </TableCell>
                      ) : null}
                    </TableRow>
                  ))}
                  {(allowancesQuery.data?.allowances.length ?? 0) === 0 ? (
                    <TableRow>
                      <TableCell
                        colSpan={canWrite ? 6 : 5}
                        className="py-10 text-center text-muted-foreground"
                      >
                        No usage allowances.
                      </TableCell>
                    </TableRow>
                  ) : null}
                </TableBody>
              </Table>
            </div>
          )}
        </TabsContent>
      </Tabs>

      <GrantDialog
        open={grantOpen}
        onOpenChange={setGrantOpen}
        form={grantForm}
        services={services}
        pending={issueGrant.isPending}
        onSubmit={submitGrant}
      />
      <AllowanceDialog
        open={allowanceOpen}
        onOpenChange={setAllowanceOpen}
        form={allowanceForm}
        services={services}
        pending={createAllowance.isPending || updateAllowance.isPending}
        editing={editingAllowance !== null}
        onSubmit={submitAllowance}
      />
      <Dialog
        open={grantToRevoke !== null}
        onOpenChange={(open) => {
          if (!open && !revokeGrant.isPending) setGrantToRevoke(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Revoke credit grant</DialogTitle>
            <DialogDescription>
              Revoke the remaining {grantToRevoke?.remaining_micros
                ? formatCredits(grantToRevoke.remaining_micros)
                : "credits"} for {grantToRevoke?.recipient_display_name ||
                grantToRevoke?.recipient_email ||
                "this recipient"}? This cannot be undone.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              disabled={revokeGrant.isPending}
              onClick={() => setGrantToRevoke(null)}
            >
              Cancel
            </Button>
            <Button
              type="button"
              variant="destructive"
              isLoading={revokeGrant.isPending}
              onClick={() => void handleRevoke()}
            >
              Revoke grant
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

type GrantFormApi = UseFormReturn<IssueGrantForm>;
type AllowanceFormApi = UseFormReturn<AllowanceForm>;

function GrantDialog({
  open,
  onOpenChange,
  form,
  services,
  pending,
  onSubmit,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly form: GrantFormApi;
  readonly services: readonly DownstreamService[];
  readonly pending: boolean;
  readonly onSubmit: (value: IssueGrantForm) => Promise<void>;
}) {
  const targetKind = form.watch("target_kind");
  const allServices = form.watch("all_services");
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>Issue credit grant</DialogTitle>
          <DialogDescription>
            Create one attributable grant per selected billing owner.
          </DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form className="space-y-5" onSubmit={form.handleSubmit(onSubmit)}>
            <div className="grid gap-4 sm:grid-cols-2">
              <FormField
                control={form.control}
                name="amount_credits"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Credits per owner</FormLabel>
                    <FormControl>
                      <Input
                        type="number"
                        min={1}
                        max={1_000_000}
                        {...field}
                        onChange={(event) =>
                          field.onChange(event.target.valueAsNumber)
                        }
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name="expires_at"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Expiry (optional)</FormLabel>
                    <FormControl>
                      <Input type="datetime-local" {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
            </div>
            <GrantTargetFields form={form} targetKind={targetKind} />
            <FormField
              control={form.control}
              name="all_services"
              render={({ field }) => (
                <FormItem className="flex items-center justify-between rounded-lg border border-border px-3 py-2">
                  <div>
                    <FormLabel>All services</FormLabel>
                    <p className="text-[11px] text-muted-foreground">
                      Allow this credit balance to fund any service.
                    </p>
                  </div>
                  <FormControl>
                    <Switch
                      checked={field.value}
                      onCheckedChange={field.onChange}
                    />
                  </FormControl>
                </FormItem>
              )}
            />
            {!allServices ? (
              <FormField
                control={form.control}
                name="service_refs"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Service scope</FormLabel>
                    <FormControl>
                      <ServicePicker
                        services={services}
                        selected={field.value}
                        onChange={field.onChange}
                        multiple
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
            ) : null}
            <FormField
              control={form.control}
              name="reason"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Reason / note</FormLabel>
                  <FormControl>
                    <textarea
                      rows={3}
                      className="w-full resize-y rounded-lg border border-input bg-transparent px-3 py-2 text-[12px] outline-none focus:border-white/15"
                      placeholder="Why these credits are being issued"
                      {...field}
                    />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
            <DialogFooter>
              <Button
                type="button"
                variant="ghost"
                onClick={() => onOpenChange(false)}
              >
                Cancel
              </Button>
              <Button type="submit" variant="primary" isLoading={pending}>
                Issue credits
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  );
}

function AllowanceDialog({
  open,
  onOpenChange,
  form,
  services,
  pending,
  editing,
  onSubmit,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly form: AllowanceFormApi;
  readonly services: readonly DownstreamService[];
  readonly pending: boolean;
  readonly editing: boolean;
  readonly onSubmit: (value: AllowanceForm) => Promise<void>;
}) {
  const targetKind = form.watch("target_kind");
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>
            {editing ? "Edit allowance" : "Create allowance"}
          </DialogTitle>
          <DialogDescription>
            Free units are consumed before credit grants and wallet credits.
          </DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form className="space-y-5" onSubmit={form.handleSubmit(onSubmit)}>
            <FormField
              control={form.control}
              name="service_ref"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Service</FormLabel>
                  <FormControl>
                    <ServicePicker
                      services={services}
                      selected={[field.value].filter(Boolean)}
                      onChange={(values) => field.onChange(values[0] ?? "")}
                    />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
            <div className="grid gap-4 sm:grid-cols-2">
              <FormField
                control={form.control}
                name="quantity"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Free units</FormLabel>
                    <FormControl>
                      <Input
                        type="number"
                        min={1}
                        max={1_000_000_000_000}
                        {...field}
                        onChange={(event) =>
                          field.onChange(event.target.valueAsNumber)
                        }
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name="recurrence"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Recurrence</FormLabel>
                    <Select value={field.value} onValueChange={field.onChange}>
                      <FormControl>
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                      </FormControl>
                      <SelectContent>
                        <SelectItem value="one_time">One time</SelectItem>
                        <SelectItem value="daily">Daily</SelectItem>
                        <SelectItem value="weekly">Weekly</SelectItem>
                        <SelectItem value="monthly">Monthly</SelectItem>
                      </SelectContent>
                    </Select>
                    <FormMessage />
                  </FormItem>
                )}
              />
            </div>
            <AllowanceTargetFields form={form} targetKind={targetKind} />
            <DialogFooter>
              <Button
                type="button"
                variant="ghost"
                onClick={() => onOpenChange(false)}
              >
                Cancel
              </Button>
              <Button type="submit" variant="primary" isLoading={pending}>
                {editing ? "Save changes" : "Create allowance"}
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  );
}

function GrantTargetFields({
  form,
  targetKind,
}: {
  readonly form: GrantFormApi;
  readonly targetKind: "all_users" | "selected_users";
}) {
  return (
    <>
      <FormField
        control={form.control}
        name="target_kind"
        render={({ field }) => (
          <FormItem>
            <FormLabel>Recipients</FormLabel>
            <Select
              value={field.value}
              onValueChange={(value) => {
                field.onChange(value);
                if (value === "all_users") form.setValue("target_user_ids", []);
              }}
            >
              <FormControl>
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
              </FormControl>
              <SelectContent>
                <SelectItem value="all_users">All billing owners</SelectItem>
                <SelectItem value="selected_users">Selected owners</SelectItem>
              </SelectContent>
            </Select>
            <FormMessage />
          </FormItem>
        )}
      />
      {targetKind === "selected_users" ? (
        <FormField
          control={form.control}
          name="target_user_ids"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Owners</FormLabel>
              <FormControl>
                <UserPicker selected={field.value} onChange={field.onChange} />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
      ) : null}
    </>
  );
}

function AllowanceTargetFields({
  form,
  targetKind,
}: {
  readonly form: AllowanceFormApi;
  readonly targetKind: "all_users" | "selected_users";
}) {
  return (
    <>
      <FormField
        control={form.control}
        name="target_kind"
        render={({ field }) => (
          <FormItem>
            <FormLabel>Recipients</FormLabel>
            <Select
              value={field.value}
              onValueChange={(value) => {
                field.onChange(value);
                if (value === "all_users") form.setValue("target_user_ids", []);
              }}
            >
              <FormControl>
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
              </FormControl>
              <SelectContent>
                <SelectItem value="all_users">All billing owners</SelectItem>
                <SelectItem value="selected_users">Selected owners</SelectItem>
              </SelectContent>
            </Select>
            <FormMessage />
          </FormItem>
        )}
      />
      {targetKind === "selected_users" ? (
        <FormField
          control={form.control}
          name="target_user_ids"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Owners</FormLabel>
              <FormControl>
                <UserPicker selected={field.value} onChange={field.onChange} />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
      ) : null}
    </>
  );
}

function UserPicker({
  selected,
  onChange,
}: {
  readonly selected: readonly string[];
  readonly onChange: (ids: string[]) => void;
}) {
  const [search, setSearch] = useState("");
  const deferredSearch = useDeferredValue(search.trim());
  const peopleQuery = useAdminUsers(
    1,
    100,
    deferredSearch || undefined,
    "person",
  );
  const organizationsQuery = useAdminUsers(
    1,
    100,
    deferredSearch || undefined,
    "org",
  );
  const users = useMemo(() => {
    const byId = new Map<string, AdminUser>();
    for (const user of [
      ...(peopleQuery.data?.users ?? []),
      ...(organizationsQuery.data?.users ?? []),
    ]) {
      if (user.is_active) byId.set(user.id, user);
    }
    return [...byId.values()];
  }, [organizationsQuery.data?.users, peopleQuery.data?.users]);
  const filtered = useMemo(() => {
    const needle = search.trim().toLowerCase();
    return needle
      ? users.filter((user) =>
          `${user.display_name ?? ""} ${user.email} ${user.slug ?? ""}`
            .toLowerCase()
            .includes(needle),
        )
      : users;
  }, [search, users]);
  const loading = peopleQuery.isFetching || organizationsQuery.isFetching;
  return (
    <div className="rounded-lg border border-border">
      <div className="relative border-b border-border">
        <Search className="absolute left-3 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
        <Input
          value={search}
          onChange={(event) => setSearch(event.target.value)}
          placeholder="Search owners"
          className="border-0 pl-9 focus-visible:ring-0"
        />
      </div>
      <div className="max-h-48 overflow-y-auto p-1">
        {filtered.map((user) => {
          const checked = selected.includes(user.id);
          return (
            <label
              key={user.id}
              className="flex cursor-pointer items-center gap-3 rounded-md px-2 py-2 hover:bg-muted/50"
            >
              <Checkbox
                checked={checked}
                onCheckedChange={(value) =>
                  onChange(
                    value === true
                      ? [...selected, user.id]
                      : selected.filter((id) => id !== user.id),
                  )
                }
              />
              <span className="min-w-0 flex-1">
                <span className="block truncate text-[12px] font-medium">
                  {user.display_name || user.slug || user.email}
                </span>
                <span className="block truncate text-[11px] text-muted-foreground">
                  {user.slug ? `Organization / ${user.slug}` : user.email}
                </span>
              </span>
            </label>
          );
        })}
        {filtered.length === 0 && !loading ? (
          <p className="px-3 py-6 text-center text-[12px] text-muted-foreground">
            No owners found.
          </p>
        ) : null}
        {loading ? (
          <p className="px-3 py-3 text-center text-[11px] text-muted-foreground">
            Searching...
          </p>
        ) : null}
      </div>
      <div className="border-t border-border px-3 py-1.5 text-[11px] text-muted-foreground">
        {selected.length} selected
      </div>
    </div>
  );
}

function ServicePicker({
  services,
  selected,
  onChange,
  multiple = false,
}: {
  readonly services: readonly DownstreamService[];
  readonly selected: readonly string[];
  readonly onChange: (ids: string[]) => void;
  readonly multiple?: boolean;
}) {
  const [search, setSearch] = useState("");
  const filtered = useMemo(() => {
    const needle = search.trim().toLowerCase();
    return services.filter(
      (service) =>
        service.is_active &&
        (!needle ||
          `${service.name} ${service.slug}`.toLowerCase().includes(needle)),
    );
  }, [search, services]);
  return (
    <div className="rounded-lg border border-border">
      <div className="relative border-b border-border">
        <Search className="absolute left-3 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
        <Input
          value={search}
          onChange={(event) => setSearch(event.target.value)}
          placeholder="Search services"
          className="border-0 pl-9 focus-visible:ring-0"
        />
      </div>
      <div className="max-h-44 overflow-y-auto p-1">
        {filtered.map((service) => {
          const checked = selected.includes(service.id);
          return (
            <label
              key={service.id}
              className="flex cursor-pointer items-center gap-3 rounded-md px-2 py-2 hover:bg-muted/50"
            >
              <Checkbox
                checked={checked}
                onCheckedChange={(value) =>
                  onChange(
                    value === true
                      ? multiple
                        ? [...selected, service.id]
                        : [service.id]
                      : selected.filter((id) => id !== service.id),
                  )
                }
              />
              <span className="min-w-0">
                <span className="block truncate text-[12px] font-medium">
                  {service.name}
                </span>
                <span className="block truncate text-[11px] text-muted-foreground">
                  {service.slug}
                </span>
              </span>
            </label>
          );
        })}
      </div>
    </div>
  );
}

function StatusBadge({
  status,
}: {
  readonly status: "active" | "consumed" | "expired" | "revoked";
}) {
  const variants = {
    active: "success",
    consumed: "secondary",
    expired: "warning",
    revoked: "destructive",
  } as const;
  return (
    <Badge variant={variants[status]} className="capitalize">
      {status}
    </Badge>
  );
}
function scopeLabel(allServices: boolean, slugs: readonly string[]) {
  return allServices
    ? "All services"
    : slugs.length <= 2
      ? slugs.join(", ")
      : `${slugs.slice(0, 2).join(", ")} +${String(slugs.length - 2)}`;
}
function serviceName(
  services: readonly DownstreamService[],
  id: string,
  slug: string,
) {
  return services.find((service) => service.id === id)?.name ?? slug;
}
function formatCredits(micros: number) {
  return `${new Intl.NumberFormat(undefined, { maximumFractionDigits: 6 }).format(micros / 1_000_000)} credits`;
}
function formatNumber(value: number) {
  return new Intl.NumberFormat().format(value);
}
function formatDateTime(value: string | null | undefined) {
  return value ? new Date(value).toLocaleString() : "Never";
}
function errorMessage(error: unknown, fallback: string) {
  return error instanceof ApiError ? error.message : fallback;
}
