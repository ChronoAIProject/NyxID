import { useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { Pencil, Plus } from "lucide-react";
import { toast } from "sonner";
import {
  useAdminAllowances,
  useAdminCreditGrants,
  useAdminCreditSchedules,
  useCreateAllowance,
  useCreateCreditSchedule,
  useIssueCreditGrant,
  useRevokeCreditGrant,
  useUpdateAllowance,
  useUpdateCreditSchedule,
} from "@/hooks/use-billing-credits";
import { useServices } from "@/hooks/use-services";
import { ApiError } from "@/lib/api-client";
import { billingMetricLabel } from "@/lib/billing-units";
import {
  allowanceFormSchema,
  issueGrantFormSchema,
  scheduleFormSchema,
  type AllowanceForm,
  type CreditSchedule,
  type CreditGrant,
  type IssueGrantForm,
  type ScheduleForm,
  type UsageAllowance,
} from "@/schemas/billing-credits";
import { useAuthStore } from "@/stores/auth-store";
import type { DownstreamService } from "@/types/api";
import { canAdminWrite } from "@/types/api";
import {
  AllowanceDialog,
  GrantDialog,
} from "@/components/admin-credits/credits-dialogs";
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
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useAppForm } from "@/components/ui/form";
import { Skeleton } from "@/components/ui/skeleton";
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

const SCHEDULE_DEFAULTS: ScheduleForm = {
  amount_credits: 100,
  recurrence: "monthly",
  expiry: { kind: "end_of_period" },
  target_kind: "all_users",
  target_user_ids: [],
  all_services: true,
  service_refs: [],
  reason: "",
};

const GRANTS_PER_PAGE = 50;

export function AdminCreditsPage() {
  const currentUser = useAuthStore((state) => state.user);
  const canWrite = canAdminWrite(currentUser);
  const [grantPage, setGrantPage] = useState(1);
  const grantsQuery = useAdminCreditGrants(grantPage, GRANTS_PER_PAGE);
  const allowancesQuery = useAdminAllowances();
  const schedulesQuery = useAdminCreditSchedules();
  const servicesQuery = useServices();
  const issueGrant = useIssueCreditGrant();
  const revokeGrant = useRevokeCreditGrant();
  const createAllowance = useCreateAllowance();
  const updateAllowance = useUpdateAllowance();
  const createSchedule = useCreateCreditSchedule();
  const updateSchedule = useUpdateCreditSchedule();
  const [grantOpen, setGrantOpen] = useState(false);
  const [allowanceOpen, setAllowanceOpen] = useState(false);
  const [scheduleOpen, setScheduleOpen] = useState(false);
  const [editingAllowance, setEditingAllowance] =
    useState<UsageAllowance | null>(null);
  const [editingSchedule, setEditingSchedule] = useState<CreditSchedule | null>(
    null,
  );
  const [grantToRevoke, setGrantToRevoke] = useState<CreditGrant | null>(null);

  const grantForm = useAppForm<IssueGrantForm>({
    resolver: zodResolver(issueGrantFormSchema),
    defaultValues: GRANT_DEFAULTS,
  });
  const allowanceForm = useAppForm<AllowanceForm>({
    resolver: zodResolver(allowanceFormSchema),
    defaultValues: ALLOWANCE_DEFAULTS,
  });
  const scheduleForm = useAppForm<ScheduleForm>({
    resolver: zodResolver(scheduleFormSchema),
    defaultValues: SCHEDULE_DEFAULTS,
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

  function openScheduleDialog(schedule?: CreditSchedule) {
    setEditingSchedule(schedule ?? null);
    scheduleForm.reset(
      schedule
        ? {
            amount_credits: schedule.amount_credits,
            recurrence: schedule.recurrence,
            expiry: schedule.expiry,
            target_kind: schedule.target_kind,
            target_user_ids: schedule.target_user_ids,
            all_services: schedule.scope.all_services,
            service_refs: schedule.scope.service_ids,
            reason: schedule.reason ?? "",
          }
        : SCHEDULE_DEFAULTS,
    );
    setScheduleOpen(true);
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
      const rolloutWarning = rolloutWarningMessage(result.recipients);
      if (rolloutWarning) toast.warning(rolloutWarning);
      setGrantPage(1);
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

  async function submitSchedule(value: ScheduleForm) {
    try {
      const targetUserIds =
        value.target_kind === "all_users" ? [] : value.target_user_ids;
      const serviceRefs = value.all_services ? [] : value.service_refs;
      if (editingSchedule) {
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
          },
        });
        toast.success("Credit schedule updated");
      } else {
        await createSchedule.mutateAsync({
          ...value,
          target_user_ids: targetUserIds,
          service_refs: serviceRefs,
        });
        toast.success("Credit schedule created");
      }
      setScheduleOpen(false);
    } catch (error) {
      toast.error(errorMessage(error, "Failed to save credit schedule"));
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

  async function toggleSchedule(schedule: CreditSchedule) {
    try {
      await updateSchedule.mutateAsync({
        id: schedule.id,
        body: { is_active: !schedule.is_active },
      });
      toast.success(
        schedule.is_active ? "Schedule paused" : "Schedule resumed",
      );
    } catch (error) {
      toast.error(errorMessage(error, "Failed to update credit schedule"));
    }
  }

  const services = servicesQuery.data ?? [];

  return (
    <div className="space-y-6">
      <PageHeader
        title="Credits"
        description="Manage promotional credit grants, recurring credit schedules, and free usage allowances."
      />

      <Tabs defaultValue="grants">
        <TabsList>
          <TabsTrigger value="grants">Credit grants</TabsTrigger>
          <TabsTrigger value="allowances">Free allowances</TabsTrigger>
          <TabsTrigger value="schedules">Schedules</TabsTrigger>
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

        <TabsContent value="schedules" className="space-y-4">
          <div className="flex items-center justify-between gap-3">
            <p className="text-[12px] text-muted-foreground">
              Disburse wallet credits on recurring UTC periods. These are
              credits, not metered service units.
            </p>
            {canWrite ? (
              <Button
                size="sm"
                variant="primary"
                onClick={() => openScheduleDialog()}
              >
                <ButtonIcon variant="primary">
                  <Plus className="h-3.5 w-3.5" />
                </ButtonIcon>
                Create schedule
              </Button>
            ) : null}
          </div>
          {schedulesQuery.isError ? (
            <ErrorBanner
              message={errorMessage(
                schedulesQuery.error,
                "Failed to load credit schedules",
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
        editingAllowance={editingAllowance}
        onSubmit={submitAllowance}
      />
      <ScheduleDialog
        open={scheduleOpen}
        onOpenChange={setScheduleOpen}
        form={scheduleForm}
        services={services}
        pending={createSchedule.isPending || updateSchedule.isPending}
        editingSchedule={editingSchedule}
        onSubmit={submitSchedule}
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

function serviceName(
  services: readonly DownstreamService[],
  id: string,
  slug: string,
) {
  return services.find((service) => service.id === id)?.name ?? slug;
}
function formatNumber(value: number) {
  return new Intl.NumberFormat().format(value);
}
function errorMessage(error: unknown, fallback: string) {
  return error instanceof ApiError ? error.message : fallback;
}
