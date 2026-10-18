import { useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { MoreHorizontal, Pencil, Plus } from "lucide-react";
import { toast } from "sonner";
import {
  useAdminAllowances,
  useAdminCreditSummary,
  useAdminCreditGrants,
  useAdminCreditSchedules,
  useCreateAllowance,
  useAddCredits,
  useRevokeCreditGrant,
  useUpdateAllowance,
  useUpdateCreditSchedule,
} from "@/hooks/use-billing-credits";
import { useServices } from "@/hooks/use-services";
import { ApiError } from "@/lib/api-client";
import { billingMetricLabel } from "@/lib/billing-units";
import {
  allowanceFormSchema,
  scheduleFormSchema,
  type AllowanceForm,
  addCreditsFormSchema,
  type AddCreditsForm,
  type CreditSchedule,
  type CreditGrant,
  type ScheduleForm,
  type UsageAllowance,
} from "@/schemas/billing-credits";
import { useAuthStore } from "@/stores/auth-store";
import type { DownstreamService } from "@/types/api";
import { canAdminWrite } from "@/types/api";
import { AllowanceDialog } from "@/components/admin-credits/credits-dialogs";
import { AddCreditsDialog } from "@/components/admin-credits/add-credits-dialog";
import { CreditGrantsTable } from "@/components/admin-credits/credit-grants-table";
import { GrantRevokeDescription } from "@/components/admin-credits/grant-revoke-description";
import { ScheduleDialog } from "@/components/admin-credits/schedule-dialog";
import { SchedulesTable } from "@/components/admin-credits/schedules-table";
import { rolloutWarningMessage } from "@/components/admin-credits/credit-grant-visibility";
import { PageHeader } from "@/components/shared/page-header";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Badge } from "@/components/ui/badge";
import { Button, ButtonIcon } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { formatMicrocredits } from "@/lib/credits";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useAppForm } from "@/components/ui/form";
import { Skeleton } from "@/components/ui/skeleton";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

const ADD_CREDITS_DEFAULTS: AddCreditsForm = {
  amount_credits: 100,
  recurrence: "one_time",
  // Require an explicit recipient choice so an untouched form cannot issue to
  // every billing owner by accident.
  target_kind: "selected_users",
  target_user_ids: [],
  all_services: true,
  service_refs: [],
  service_limits: [],
  expires_at: "",
  expiry: { kind: "end_of_period" },
  reason: "",
};

const ALLOWANCE_DEFAULTS: AllowanceForm = {
  service_ref: "",
  quantity: 1_000,
  recurrence: "monthly",
  target_kind: "all_users",
  target_user_ids: [],
};

const SCHEDULE_DEFAULTS: ScheduleForm = {
  amount_credits: 100,
  recurrence: "monthly",
  expiry: { kind: "end_of_period" },
  target_kind: "all_users",
  target_user_ids: [],
  all_services: true,
  service_refs: [],
  service_limits: [],
  reason: "",
};

const GRANTS_PER_PAGE = 50;

export function AdminCreditsPage() {
  const currentUser = useAuthStore((state) => state.user);
  const isAdmin = canAdminWrite(currentUser);
  const summaryQuery = useAdminCreditSummary();
  // Older backends may accept grant writes while ignoring new policy fields.
  // Require a successfully validated summary before exposing any credit edits.
  const canWrite =
    isAdmin && Boolean(summaryQuery.data) && !summaryQuery.isError;
  const [grantPage, setGrantPage] = useState(1);
  const [activeTab, setActiveTab] = useState("grants");
  const grantsQuery = useAdminCreditGrants(grantPage, GRANTS_PER_PAGE);
  const allowancesQuery = useAdminAllowances();
  const schedulesQuery = useAdminCreditSchedules();
  const servicesQuery = useServices();
  const addCredits = useAddCredits();
  const revokeGrant = useRevokeCreditGrant();
  const createAllowance = useCreateAllowance();
  const updateAllowance = useUpdateAllowance();
  const updateSchedule = useUpdateCreditSchedule();
  const [addCreditsOpen, setAddCreditsOpen] = useState(false);
  const [allowanceOpen, setAllowanceOpen] = useState(false);
  const [scheduleOpen, setScheduleOpen] = useState(false);
  const [editingAllowance, setEditingAllowance] =
    useState<UsageAllowance | null>(null);
  const [editingSchedule, setEditingSchedule] = useState<CreditSchedule | null>(
    null,
  );
  const [grantToRevoke, setGrantToRevoke] = useState<CreditGrant | null>(null);

  const addCreditsForm = useAppForm<AddCreditsForm>({
    mode: "onChange",
    resolver: zodResolver(addCreditsFormSchema),
    defaultValues: ADD_CREDITS_DEFAULTS,
  });
  const allowanceForm = useAppForm<AllowanceForm>({
    mode: "onChange",
    resolver: zodResolver(allowanceFormSchema),
    defaultValues: ALLOWANCE_DEFAULTS,
  });
  const scheduleForm = useAppForm<ScheduleForm>({
    mode: "onChange",
    resolver: zodResolver(scheduleFormSchema),
    defaultValues: SCHEDULE_DEFAULTS,
  });

  function openAddCreditsDialog(
    recurrence: AddCreditsForm["recurrence"] = "one_time",
  ) {
    if (!canWrite || addCredits.isPending) return;
    addCredits.reset();
    addCreditsForm.reset({ ...ADD_CREDITS_DEFAULTS, recurrence });
    setAddCreditsOpen(true);
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

  function openScheduleDialog(schedule: CreditSchedule) {
    setEditingSchedule(schedule);
    scheduleForm.reset({
      amount_credits: schedule.amount_credits,
      recurrence: schedule.recurrence,
      expiry: schedule.expiry,
      target_kind: schedule.target_kind,
      target_user_ids: schedule.target_user_ids,
      all_services: schedule.scope.all_services,
      service_refs: schedule.scope.service_ids,
      service_limits: (schedule.service_limits ?? []).map((rule) => ({
        service_refs: rule.service_ids,
        amount_credits: Number(BigInt(rule.limit_micros) / 1_000_000n),
        label: rule.label ?? "",
      })),
      reason: schedule.reason ?? "",
    });
    setScheduleOpen(true);
  }

  async function submitAddCredits(value: AddCreditsForm) {
    if (!canWrite) return;
    try {
      const result = await addCredits.mutateAsync(value);
      if ("created_count" in result) {
        const recipientLabel = `${String(result.created_count)} owner${result.created_count === 1 ? "" : "s"}`;
        toast.success(
          result.pending_activation_count > 0
            ? `Added credits for ${recipientLabel}; ${String(result.pending_activation_count)} pending activation`
            : `Added credits for ${recipientLabel}`,
        );
        const rolloutWarning = rolloutWarningMessage(result.recipients);
        if (rolloutWarning) toast.warning(rolloutWarning);
      } else {
        toast.success(
          value.target_kind === "all_users"
            ? "Recurring credits added; the current UTC period is issued on the next billing update, with billing owners captured when the period starts"
            : "Recurring credits added; the current UTC period is issued on the next billing update",
        );
      }
      setGrantPage(1);
      setActiveTab(value.recurrence === "one_time" ? "grants" : "schedules");
      setAddCreditsOpen(false);
    } catch (error) {
      toast.error(errorMessage(error, "Failed to add credits"));
    }
  }

  async function submitAllowance(value: AllowanceForm) {
    if (!canWrite) return;
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
        toast.success("Included usage updated");
      } else {
        await createAllowance.mutateAsync(normalized);
        toast.success("Included usage added");
      }
      setAllowanceOpen(false);
    } catch (error) {
      toast.error(errorMessage(error, "Failed to save included usage"));
    }
  }

  async function submitSchedule(value: ScheduleForm) {
    if (!canWrite || !editingSchedule) return;
    try {
      const targetUserIds =
        value.target_kind === "all_users" ? [] : value.target_user_ids;
      const serviceRefs = value.all_services ? [] : value.service_refs;
      await updateSchedule.mutateAsync({
        id: editingSchedule.id,
        body: {
          amount_credits: value.amount_credits,
          expiry: value.expiry,
          target_kind: value.target_kind,
          target_user_ids: targetUserIds,
          all_services: value.all_services,
          service_refs: serviceRefs,
          reason: value.reason,
          service_limits: value.service_limits ?? [],
        },
      });
      toast.success("Recurring credits updated");
      setScheduleOpen(false);
    } catch (error) {
      toast.error(errorMessage(error, "Failed to save recurring credits"));
    }
  }

  async function handleRevoke() {
    if (!canWrite || !grantToRevoke || revokeGrant.isPending) return;
    try {
      const result = await revokeGrant.mutateAsync(grantToRevoke.id);
      toast.success(
        result.status === "active" && result.revoke_requested_at
          ? "Reclaim requested; held requests can finish before the remainder is removed"
          : "Remaining credits reclaimed",
      );
      setGrantToRevoke(null);
    } catch (error) {
      toast.error(errorMessage(error, "Failed to reclaim remaining credits"));
    }
  }

  async function toggleAllowance(allowance: UsageAllowance) {
    if (!canWrite) return;
    try {
      await updateAllowance.mutateAsync({
        id: allowance.id,
        body: { is_active: !allowance.is_active },
      });
      toast.success(
        allowance.is_active
          ? "Included usage disabled"
          : "Included usage enabled",
      );
    } catch (error) {
      toast.error(errorMessage(error, "Failed to update included usage"));
    }
  }

  async function toggleSchedule(schedule: CreditSchedule) {
    if (!canWrite) return;
    try {
      await updateSchedule.mutateAsync({
        id: schedule.id,
        body: { is_active: !schedule.is_active },
      });
      toast.success(
        schedule.is_active
          ? "Recurring credits paused"
          : "Recurring credits resumed",
      );
    } catch (error) {
      toast.error(errorMessage(error, "Failed to update recurring credits"));
    }
  }

  const services = servicesQuery.data ?? [];

  return (
    <div className="space-y-6">
      <PageHeader
        title="Credits"
        description="Choose who receives credits, where they can spend them, and when they expire."
        actions={
          isAdmin ? (
            <Button
              variant="primary"
              disabled={!canWrite}
              onClick={() => openAddCreditsDialog()}
            >
              <Plus className="mr-1.5 h-4 w-4" />
              Allocate credits
            </Button>
          ) : undefined
        }
      />
      <section
        aria-label="Credit balance"
        className="rounded-lg border border-border px-4 py-3"
      >
        <div className="flex flex-wrap items-center justify-between gap-x-6 gap-y-2">
          <div>
            <div className="flex items-center gap-2 text-[12px] text-muted-foreground">
              Credits remaining
              {summaryQuery.isError && summaryQuery.data ? (
                <Badge variant="warning">Stale</Badge>
              ) : null}
            </div>
            <p className="mt-1 [overflow-wrap:anywhere] text-[24px] font-semibold leading-tight">
              {summaryQuery.data
                ? formatMicrocredits(summaryQuery.data.outstanding_micros)
                : "—"}
            </p>
          </div>
          {summaryQuery.data ? (
            <p className="text-[12px] text-muted-foreground">
              {formatNumber(summaryQuery.data.active_grant_count)} active
              allocation{summaryQuery.data.active_grant_count === 1 ? "" : "s"}{" "}
              · {formatNumber(summaryQuery.data.recipient_count)} recipient
              {summaryQuery.data.recipient_count === 1 ? "" : "s"}
            </p>
          ) : null}
        </div>
        {summaryQuery.isError ? (
          <ErrorBanner
            message={
              summaryQuery.error instanceof ApiError &&
              summaryQuery.error.status === 404
                ? "This backend does not support this Credits page. Update the backend before allocating, editing, or reclaiming credits."
                : `${errorMessage(summaryQuery.error, "Failed to load credit summary")}. Credit changes are unavailable until the summary loads successfully.`
            }
            onRetry={() => void summaryQuery.refetch()}
          />
        ) : summaryQuery.isLoading ? (
          <Skeleton className="mt-2 h-4 w-48" />
        ) : summaryQuery.data ? (
          <details className="mt-2 text-[12px]">
            <summary className="w-fit cursor-pointer text-muted-foreground">
              View balance details
            </summary>
            <dl className="mt-2 grid max-w-xl gap-x-6 gap-y-1 sm:grid-cols-2">
              <SummaryMetric
                label="Available to use"
                value={formatMicrocredits(summaryQuery.data.available_micros)}
              />
              <SummaryMetric
                label="In use by active requests"
                value={formatMicrocredits(summaryQuery.data.reserved_micros)}
              />
              <SummaryMetric
                label="Awaiting activation"
                value={formatMicrocredits(
                  summaryQuery.data.pending_activation_micros,
                )}
              />
              <SummaryMetric
                label="Currently unavailable"
                value={formatMicrocredits(
                  summaryQuery.data.unavailable_micros ?? "0",
                )}
              />
              <SummaryMetric
                label="Expires within 30 days"
                value={formatMicrocredits(
                  summaryQuery.data.expiring_within_30_days_micros,
                )}
              />
            </dl>
            <p className="mt-2 text-[11px] text-muted-foreground">
              Issued credits only. Each allocation's service limits and expiry
              apply.
            </p>
          </details>
        ) : null}
      </section>

      <Tabs value={activeTab} onValueChange={setActiveTab}>
        <TabsList>
          <TabsTrigger value="grants">Credit grants</TabsTrigger>
          <TabsTrigger value="allowances">Free allowances</TabsTrigger>
          <TabsTrigger value="schedules">Schedules</TabsTrigger>
        </TabsList>
        <TabsContent value="grants" className="space-y-4">
          <p className="text-[12px] text-muted-foreground">
            Each allocation has its own balance, service limits, and expiry.
            Reclaim its unused credits at any time.
          </p>

          {grantsQuery.isError ? (
            <ErrorBanner
              message={errorMessage(
                grantsQuery.error,
                "Failed to load issued credits",
              )}
              onRetry={() => void grantsQuery.refetch()}
            />
          ) : grantsQuery.isLoading ? (
            <Skeleton className="h-52 w-full" />
          ) : (
            <CreditGrantsTable
              grants={grantsQuery.data?.grants ?? []}
              canWrite={canWrite}
              revokePending={revokeGrant.isPending}
              page={grantsQuery.data?.page ?? grantPage}
              perPage={grantsQuery.data?.per_page ?? GRANTS_PER_PAGE}
              total={grantsQuery.data?.total ?? 0}
              fetching={grantsQuery.isFetching}
              onPageChange={setGrantPage}
              onRevoke={(grant) => setGrantToRevoke(grant)}
            />
          )}
        </TabsContent>
        <TabsContent value="allowances" className="space-y-4">
          <div className="flex items-center justify-between gap-3">
            <p className="text-[12px] text-muted-foreground">
              Set free usage in requests, tokens, or bytes. Recurring allowances
              reset on UTC periods.
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
                "Failed to load included usage",
              )}
              onRetry={() => void allowancesQuery.refetch()}
            />
          ) : allowancesQuery.isLoading ? (
            <Skeleton className="h-52 w-full" />
          ) : (
            <>
              <div className="hidden overflow-x-auto rounded-lg border border-border sm:block">
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
                    {(allowancesQuery.data?.allowances ?? []).map(
                      (allowance) => (
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
                              {allowance.service_slug}
                            </div>
                          </TableCell>
                          <TableCell>
                            {formatNumber(allowance.quantity)}{" "}
                            <span className="text-[11px] text-muted-foreground">
                              {billingMetricLabel(
                                allowance.metric,
                                allowance.quantity,
                              )}
                            </span>
                          </TableCell>
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
                              <DropdownMenu>
                                <DropdownMenuTrigger asChild>
                                  <Button
                                    type="button"
                                    size="icon"
                                    variant="ghost"
                                    aria-label="Included usage actions"
                                  >
                                    <MoreHorizontal className="h-4 w-4" />
                                  </Button>
                                </DropdownMenuTrigger>
                                <DropdownMenuContent align="end">
                                  <DropdownMenuItem
                                    onSelect={() =>
                                      openAllowanceDialog(allowance)
                                    }
                                  >
                                    <Pencil /> Edit included usage
                                  </DropdownMenuItem>
                                  <DropdownMenuItem
                                    disabled={updateAllowance.isPending}
                                    onSelect={() =>
                                      void toggleAllowance(allowance)
                                    }
                                  >
                                    {allowance.is_active
                                      ? "Disable included usage"
                                      : "Enable included usage"}
                                  </DropdownMenuItem>
                                </DropdownMenuContent>
                              </DropdownMenu>
                            </TableCell>
                          ) : null}
                        </TableRow>
                      ),
                    )}
                    {(allowancesQuery.data?.allowances.length ?? 0) === 0 ? (
                      <TableRow>
                        <TableCell
                          colSpan={canWrite ? 6 : 5}
                          className="py-10 text-center text-muted-foreground"
                        >
                          No included usage yet.
                        </TableCell>
                      </TableRow>
                    ) : null}
                  </TableBody>
                </Table>
              </div>
              <div className="space-y-2 sm:hidden">
                {(allowancesQuery.data?.allowances ?? []).map((allowance) => (
                  <article
                    key={allowance.id}
                    className="space-y-3 rounded-lg border border-border p-3"
                  >
                    <div className="flex items-start justify-between gap-3">
                      <div className="min-w-0">
                        <p className="break-words text-[12px] font-medium">
                          {serviceName(
                            services,
                            allowance.service_id,
                            allowance.service_slug,
                          )}
                        </p>
                        <p className="break-words text-[11px] text-muted-foreground">
                          {allowance.service_slug}
                        </p>
                      </div>
                      <Badge
                        variant={allowance.is_active ? "success" : "secondary"}
                      >
                        {allowance.is_active ? "Active" : "Disabled"}
                      </Badge>
                    </div>
                    <div className="grid grid-cols-2 gap-2 text-[11px] text-muted-foreground">
                      <span>
                        Quantity: {formatNumber(allowance.quantity)}{" "}
                        {billingMetricLabel(
                          allowance.metric,
                          allowance.quantity,
                        )}
                      </span>
                      <span>
                        Recurrence: {allowance.recurrence.replace("_", " ")}
                      </span>
                      <span>
                        Targets:{" "}
                        {allowance.target_kind === "all_users"
                          ? "All owners"
                          : `${String(allowance.target_user_ids.length)} selected`}
                      </span>
                    </div>
                    {canWrite ? (
                      <div className="flex justify-end">
                        <DropdownMenu>
                          <DropdownMenuTrigger asChild>
                            <Button
                              type="button"
                              size="sm"
                              variant="outline"
                              aria-label="Included usage actions"
                            >
                              Actions
                            </Button>
                          </DropdownMenuTrigger>
                          <DropdownMenuContent align="end">
                            <DropdownMenuItem
                              onSelect={() => openAllowanceDialog(allowance)}
                            >
                              <Pencil /> Edit included usage
                            </DropdownMenuItem>
                            <DropdownMenuItem
                              disabled={updateAllowance.isPending}
                              onSelect={() => void toggleAllowance(allowance)}
                            >
                              {allowance.is_active
                                ? "Disable included usage"
                                : "Enable included usage"}
                            </DropdownMenuItem>
                          </DropdownMenuContent>
                        </DropdownMenu>
                      </div>
                    ) : null}
                  </article>
                ))}
                {(allowancesQuery.data?.allowances.length ?? 0) === 0 ? (
                  <div className="rounded-lg border border-border py-10 text-center text-muted-foreground">
                    No included usage yet.
                  </div>
                ) : null}
              </div>
            </>
          )}
        </TabsContent>
        <TabsContent value="schedules" className="space-y-4">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <p className="text-[12px] text-muted-foreground">
              Issue fresh credits daily, weekly, or monthly. Edit future
              allocations or pause issuance.
            </p>
            {isAdmin ? (
              <Button
                size="sm"
                variant="primary"
                disabled={!canWrite}
                onClick={() => openAddCreditsDialog("monthly")}
              >
                <Plus className="mr-1.5 h-3.5 w-3.5" />
                Create schedule
              </Button>
            ) : null}
          </div>

          {schedulesQuery.isError ? (
            <ErrorBanner
              message={errorMessage(
                schedulesQuery.error,
                "Failed to load recurring credits",
              )}
              onRetry={() => void schedulesQuery.refetch()}
            />
          ) : schedulesQuery.isLoading ? (
            <Skeleton className="h-52 w-full" />
          ) : (
            <SchedulesTable
              schedules={schedulesQuery.data?.schedules ?? []}
              canWrite={canWrite}
              updatePending={updateSchedule.isPending}
              onEdit={openScheduleDialog}
              onToggle={(schedule) => void toggleSchedule(schedule)}
            />
          )}
        </TabsContent>
      </Tabs>

      <AddCreditsDialog
        open={addCreditsOpen && canWrite}
        onOpenChange={setAddCreditsOpen}
        form={addCreditsForm}
        services={services}
        servicesLoading={servicesQuery.isLoading || servicesQuery.isFetching}
        servicesError={Boolean(servicesQuery.error)}
        onRetryServices={() => void servicesQuery.refetch()}
        pending={addCredits.isPending}
        onSubmit={submitAddCredits}
      />
      <AllowanceDialog
        open={allowanceOpen && canWrite}
        onOpenChange={setAllowanceOpen}
        form={allowanceForm}
        services={services}
        servicesLoading={servicesQuery.isLoading || servicesQuery.isFetching}
        servicesError={Boolean(servicesQuery.error)}
        onRetryServices={() => void servicesQuery.refetch()}
        pending={createAllowance.isPending || updateAllowance.isPending}
        editingAllowance={editingAllowance}
        onSubmit={submitAllowance}
      />
      {editingSchedule ? (
        <ScheduleDialog
          open={scheduleOpen && canWrite}
          onOpenChange={setScheduleOpen}
          form={scheduleForm}
          services={services}
          servicesLoading={servicesQuery.isLoading || servicesQuery.isFetching}
          servicesError={Boolean(servicesQuery.error)}
          onRetryServices={() => void servicesQuery.refetch()}
          pending={updateSchedule.isPending}
          editingSchedule={editingSchedule}
          onSubmit={submitSchedule}
        />
      ) : null}
      <Dialog
        open={grantToRevoke !== null && canWrite}
        onOpenChange={(open) => {
          if (!open && !revokeGrant.isPending) setGrantToRevoke(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Reclaim remaining credits</DialogTitle>
            {grantToRevoke ? (
              <GrantRevokeDescription grant={grantToRevoke} />
            ) : null}
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
              disabled={revokeGrant.isPending}
              isLoading={revokeGrant.isPending}
              onClick={() => void handleRevoke()}
            >
              Reclaim remaining credits
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

function serviceName(
  services: readonly DownstreamService[],
  id: string,
  slug: string,
) {
  return services.find((service) => service.id === id)?.name ?? slug;
}
function SummaryMetric({
  label,
  value,
}: {
  readonly label: string;
  readonly value: string;
}) {
  return (
    <div className="flex justify-between gap-3 py-1">
      <dt className="text-muted-foreground">{label}</dt>
      <dd className="text-right font-medium tabular-nums">{value}</dd>
    </div>
  );
}

function formatNumber(value: number) {
  return new Intl.NumberFormat().format(value);
}
function errorMessage(error: unknown, fallback: string) {
  return error instanceof ApiError ? error.message : fallback;
}
