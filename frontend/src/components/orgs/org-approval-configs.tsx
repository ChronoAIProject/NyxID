import { useMemo, useState } from "react";
import { Shield, RotateCcw, Trash2 } from "lucide-react";
import { ErrorBanner } from "@/components/shared/error-banner";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Switch } from "@/components/ui/switch";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { ApiError } from "@/lib/api-client";
import {
  useServiceApprovalConfigs,
  useSetServiceApprovalConfig,
  useDeleteServiceApprovalConfig,
  useApprovalGrants,
  useRevokeGrant,
} from "@/hooks/use-approvals";
import { useKeys } from "@/hooks/use-keys";
import { formatDate } from "@/lib/utils";
import type { ApprovalMode } from "@/types/approvals";

interface OrgApprovalConfigsProps {
  readonly orgId: string;
}

/**
 * Admin-only UI for setting per-service approval policies on org-owned
 * services. When a policy is set on an org service, the org's policy is
 * dominant over any actor's personal gate: every member of the org will
 * require approval by the primary admin(s) of the org before their proxy
 * call goes through.
 */
export function OrgApprovalConfigs({ orgId }: OrgApprovalConfigsProps) {
  const { data: keys, isLoading: isKeysLoading } = useKeys();
  const {
    data: serviceConfigs,
    isLoading: isConfigsLoading,
    error: configsError,
    refetch: refetchConfigs,
  } = useServiceApprovalConfigs(orgId);
  const setConfigMutation = useSetServiceApprovalConfig();
  const deleteConfigMutation = useDeleteServiceApprovalConfig();

  const [addDialogOpen, setAddDialogOpen] = useState(false);
  const [selectedServiceId, setSelectedServiceId] = useState("");
  const [selectedApprovalRequired, setSelectedApprovalRequired] =
    useState(true);
  const [selectedApprovalMode, setSelectedApprovalMode] =
    useState<ApprovalMode>("per_request");

  // Only org-owned services for this org -- personal services show up under
  // /notifications. We also hide disabled keys so admins do not configure
  // a policy on a service that no-one can use. Custom services (no
  // `catalog_service_id`) are included: the approval config endpoint
  // accepts UserService IDs directly, so custom org services are
  // configurable too (ChronoAIProject/NyxID#165).
  const orgServices = useMemo(() => {
    return (keys ?? []).filter((key) => {
      const src = key.credential_source;
      return (
        key.is_active &&
        src !== undefined &&
        src.type === "org" &&
        src.org_id === orgId
      );
    });
  }, [keys, orgId]);

  // Identifier set used to dedupe the picker. A catalog-backed config is
  // keyed by `catalog_service_id` and covers every org UserService that
  // shares that catalog id; a custom-service config is keyed by the
  // UserService id directly. Collect both so the Add dialog hides every
  // service already covered, not just the one `user_service_id` the
  // backend returned for catalog-backed configs (which is the
  // most-recently-created sibling — the others would otherwise still
  // appear and silently overwrite the same catalog-wide policy).
  const configuredIds = useMemo(() => {
    const ids = new Set<string>();
    for (const c of serviceConfigs?.configs ?? []) {
      ids.add(c.service_id);
      if (c.user_service_id) ids.add(c.user_service_id);
    }
    return ids;
  }, [serviceConfigs]);

  const availableServices = useMemo(
    () =>
      orgServices.filter((s) => {
        if (configuredIds.has(s.id)) return false;
        if (s.catalog_service_id && configuredIds.has(s.catalog_service_id))
          return false;
        return true;
      }),
    [orgServices, configuredIds],
  );

  async function handleAdd() {
    if (!selectedServiceId) return;
    try {
      await setConfigMutation.mutateAsync({
        serviceId: selectedServiceId,
        approvalRequired: selectedApprovalRequired,
        approvalMode: selectedApprovalMode,
        orgId,
      });
      toast.success("Org approval policy added");
      setAddDialogOpen(false);
      setSelectedServiceId("");
      setSelectedApprovalRequired(true);
      setSelectedApprovalMode("per_request");
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to add policy",
      );
    }
  }

  async function handleToggle(serviceId: string, approvalRequired: boolean) {
    try {
      // `serviceId` is the mutation key (`user_service_id` when known).
      // Match against either field so catalog-keyed configs resolve too.
      const existing = serviceConfigs?.configs.find(
        (c) =>
          c.user_service_id === serviceId || c.service_id === serviceId,
      );
      await setConfigMutation.mutateAsync({
        serviceId,
        approvalRequired,
        approvalMode: existing?.approval_mode,
        orgId,
      });
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to update policy",
      );
    }
  }

  async function handleChangeMode(
    serviceId: string,
    approvalRequired: boolean,
    approvalMode: ApprovalMode,
  ) {
    try {
      await setConfigMutation.mutateAsync({
        serviceId,
        approvalRequired,
        approvalMode,
        orgId,
      });
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to update approval mode",
      );
    }
  }

  async function handleDelete(serviceId: string) {
    try {
      await deleteConfigMutation.mutateAsync({ serviceId, orgId });
      toast.success("Org approval policy removed");
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to remove policy",
      );
    }
  }

  const loading = isKeysLoading || isConfigsLoading;

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <div>
              <CardTitle className="flex items-center gap-2">
                <span className="flex h-8 w-8 items-center justify-center rounded-[8px] border border-white/[0.08] bg-white/[0.04]">
                  <Shield className="h-4 w-4" aria-hidden="true" />
                </span>
                Org Approval Policies
              </CardTitle>
              <CardDescription>
                Require approval when members of this org use shared services.
                When an org policy is set, it takes precedence over the
                member&rsquo;s personal approval settings, and admins of this
                org are notified to decide.
              </CardDescription>
            </div>
            <Button
              variant="outline"
              onClick={() => setAddDialogOpen(true)}
              disabled={
                loading ||
                Boolean(configsError) ||
                availableServices.length === 0
              }
            >
              Add Policy
            </Button>
          </div>
        </CardHeader>
        <CardContent>
          {loading ? (
            <div className="space-y-3 py-2">
              <Skeleton className="h-16 w-full" />
              <Skeleton className="h-16 w-full" />
            </div>
          ) : configsError ? (
            <div className="py-4">
              <ErrorBanner message="Failed to load org approval policies. Try refreshing the page." onRetry={refetchConfigs} />
            </div>
          ) : orgServices.length === 0 ? (
            <div className="rounded-lg bg-white/[0.03] px-4 py-3 text-[12px] text-muted-foreground">
              No org-owned services yet. Add a key to this org before
              configuring approval policies.
            </div>
          ) : serviceConfigs?.configs.length === 0 ? (
            <div className="rounded-lg bg-white/[0.03] px-4 py-3 text-[12px] text-muted-foreground">
              No org approval policies configured. Members use their personal
              approval settings.
            </div>
          ) : (
            <div className="space-y-3">
              {serviceConfigs?.configs.map((config) => {
                // Prefer the UserService id so the backend resolves the
                // same row admins see on the Keys page. Falls back to the
                // raw service_id for legacy rows whose backing
                // UserService has been deleted.
                const mutationKey =
                  config.user_service_id ?? config.service_id;
                return (
                <div
                  key={config.service_id}
                  className="rounded-lg border border-border p-4"
                >
                  <div className="flex items-center justify-between">
                    <div className="space-y-0.5">
                      <p className="text-[12px] font-medium">
                        {config.service_name}
                      </p>
                      <p className="text-xs text-muted-foreground">
                        {config.approval_required
                          ? "Approval required (org policy dominant)"
                          : "Approval not required (org policy)"}
                      </p>
                    </div>
                    <div className="flex items-center gap-3">
                      <Switch
                        checked={config.approval_required}
                        onCheckedChange={(checked) =>
                          void handleToggle(mutationKey, checked)
                        }
                      />
                      <Button
                        variant="ghost"
                        size="icon"
                        onClick={() => void handleDelete(mutationKey)}
                        title="Remove org policy"
                      >
                        <RotateCcw className="h-4 w-4 text-muted-foreground" />
                      </Button>
                    </div>
                  </div>
                  {config.approval_required && (
                    <div className="mt-3 flex items-center gap-2 border-t border-border pt-3">
                      <span className="text-xs text-muted-foreground">
                        Mode:
                      </span>
                      <Select
                        value={config.approval_mode}
                        onValueChange={(value) =>
                          void handleChangeMode(
                            mutationKey,
                            config.approval_required,
                            value as ApprovalMode,
                          )
                        }
                      >
                        <SelectTrigger className="h-7 w-[180px] text-xs">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="per_request">
                            Per request
                          </SelectItem>
                          <SelectItem value="grant">
                            Time-based grant
                          </SelectItem>
                        </SelectContent>
                      </Select>
                      <span className="text-xs text-muted-foreground">
                        {config.approval_mode === "grant"
                          ? "Approval creates a reusable grant"
                          : "Every request needs fresh approval"}
                      </span>
                    </div>
                  )}
                </div>
                );
              })}
            </div>
          )}
        </CardContent>
      </Card>

      <OrgApprovalGrants orgId={orgId} />

      <Dialog open={addDialogOpen} onOpenChange={setAddDialogOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Add Org Approval Policy</DialogTitle>
            <DialogDescription>
              Choose an org-owned service and configure whether approval is
              required for any member using it.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-4 py-4">
            <div className="space-y-2">
              <label
                className="text-[12px] font-medium"
                htmlFor="org-service-select"
              >
                Service
              </label>
              <Select
                value={selectedServiceId}
                onValueChange={setSelectedServiceId}
              >
                <SelectTrigger id="org-service-select">
                  <SelectValue placeholder="Select a service" />
                </SelectTrigger>
                <SelectContent>
                  {availableServices.map((s) => (
                    // Use the `UserService.id` as the option value. The
                    // backend resolves it to the effective storage key
                    // (catalog id for catalog-backed services, user
                    // service id for custom ones) so this works for both
                    // — in particular, custom org services become
                    // configurable here (ChronoAIProject/NyxID#165).
                    <SelectItem key={s.id} value={s.id}>
                      {s.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="flex items-center justify-between rounded-lg border border-border p-4">
              <div className="space-y-0.5">
                <p className="text-[12px] font-medium">Require Approval</p>
                <p className="text-xs text-muted-foreground">
                  Whether members must get an admin&rsquo;s approval to use
                  this service.
                </p>
              </div>
              <Switch
                checked={selectedApprovalRequired}
                onCheckedChange={setSelectedApprovalRequired}
              />
            </div>
            {selectedApprovalRequired && (
              <div className="space-y-2">
                <label
                  className="text-[12px] font-medium"
                  htmlFor="org-approval-mode-select"
                >
                  Approval Mode
                </label>
                <Select
                  value={selectedApprovalMode}
                  onValueChange={(v) =>
                    setSelectedApprovalMode(v as ApprovalMode)
                  }
                >
                  <SelectTrigger id="org-approval-mode-select">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="per_request">Per request</SelectItem>
                    <SelectItem value="grant">Time-based grant</SelectItem>
                  </SelectContent>
                </Select>
                <p className="text-xs text-muted-foreground">
                  {selectedApprovalMode === "grant"
                    ? "Approval creates a reusable grant lasting the configured number of days. Subsequent requests skip approval until the grant expires."
                    : "Every API request requires a fresh approval. No grant is created."}
                </p>
              </div>
            )}
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setAddDialogOpen(false)}>
              Cancel
            </Button>
            <Button
              variant="primary"
              onClick={() => void handleAdd()}
              disabled={!selectedServiceId}
              isLoading={setConfigMutation.isPending}
            >
              Add Policy
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

interface OrgApprovalGrantsProps {
  readonly orgId: string;
}

/**
 * Org-scoped grants list. Grants are only created when an org-policy
 * approval is decided in `grant` mode -- they live under the org's
 * user_id, so without this admin-only panel they would be unreachable.
 */
function OrgApprovalGrants({ orgId }: OrgApprovalGrantsProps) {
  const { data, isLoading, error, refetch } = useApprovalGrants(1, 50, orgId);
  const revokeMutation = useRevokeGrant();
  const grants = data?.grants ?? [];

  async function handleRevoke(grantId: string) {
    try {
      await revokeMutation.mutateAsync({ grantId, orgId });
      toast.success("Grant revoked");
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to revoke grant",
      );
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <span className="flex h-8 w-8 items-center justify-center rounded-[8px] border border-white/[0.08] bg-white/[0.04]">
            <Shield className="h-4 w-4" aria-hidden="true" />
          </span>
          Org Approval Grants
        </CardTitle>
        <CardDescription>
          Active reusable grants created when org approval policies in
          time-based grant mode have been decided. Revoking a grant forces
          the next member call to require fresh approval.
        </CardDescription>
      </CardHeader>
      <CardContent>
        {isLoading ? (
          <div className="space-y-3 py-2">
            <Skeleton className="h-14 w-full" />
            <Skeleton className="h-14 w-full" />
          </div>
        ) : error ? (
          <div className="py-4">
            <ErrorBanner message="Failed to load org approval grants. Try refreshing the page." onRetry={refetch} />
          </div>
        ) : grants.length === 0 ? (
          <div className="rounded-lg bg-white/[0.03] px-4 py-3 text-[12px] text-muted-foreground">
            No active org approval grants.
          </div>
        ) : (
          <div className="space-y-3">
            {grants.map((grant) => (
              <div
                key={grant.id}
                className="flex items-center justify-between rounded-lg border border-border p-4"
              >
                <div className="space-y-0.5">
                  <p className="text-[12px] font-medium">{grant.service_name}</p>
                  <p className="text-xs text-muted-foreground">
                    {grant.requester_label ?? grant.requester_type} ·
                    granted {formatDate(grant.granted_at)} · expires{" "}
                    {formatDate(grant.expires_at)}
                  </p>
                </div>
                <Button
                  variant="ghost"
                  size="icon"
                  onClick={() => void handleRevoke(grant.id)}
                  title="Revoke grant"
                >
                  <Trash2 className="h-4 w-4 text-destructive" />
                </Button>
              </div>
            ))}
          </div>
        )}
      </CardContent>
    </Card>
  );
}
