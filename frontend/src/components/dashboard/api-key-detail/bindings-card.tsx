import { useState } from "react";
import {
  useAgentBindings,
  useCreateBinding,
  useDeleteBinding,
} from "@/hooks/use-agent-bindings";
import { useUpdateApiKey } from "@/hooks/use-api-keys";
import { useKeys } from "@/hooks/use-keys";
import { ApiError } from "@/lib/api-client";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
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
import { AlertTriangle, Link2, Plus, Trash2 } from "lucide-react";
import { CrystalLatticeIcon } from "@/components/icons/empty-state";
import { toast } from "sonner";
import type { AgentServiceBinding } from "@/types/keys";
import type { CredentialSource } from "@/schemas/orgs";

/// True iff two credential sources point at the same owner. Used by the
/// binding / scope pickers to filter the service list to options the
/// backend will actually accept (`agent_binding_service::create_binding`
/// requires the api_key, user_service, and user_api_key to share a
/// `user_id`, so an org-owned agent key can only bind to that org's
/// services -- never to personal services or to a different org).
function sameOwner(
  a?: CredentialSource,
  b?: CredentialSource,
): boolean {
  const aType = a?.type ?? "personal";
  const bType = b?.type ?? "personal";
  if (aType !== bType) return false;
  if (aType === "personal") return true;
  return a?.type === "org" && b?.type === "org" && a.org_id === b.org_id;
}

function invalidReasonLabel(reason: string | undefined): string {
  switch (reason) {
    case "missing_service":
      return "Bound service has been deleted.";
    case "inactive_service":
      return "Bound service is disabled.";
    case "missing_credential":
      return "Override credential has been deleted.";
    default:
      return "This binding is no longer valid.";
  }
}

export function BindingsCard({
  keyId,
  allowAllServices,
  apiKeySource,
}: {
  readonly keyId: string;
  readonly allowAllServices: boolean;
  readonly apiKeySource?: CredentialSource;
}) {
  const { data: bindings, isLoading } = useAgentBindings(keyId);
  const { data: allKeys } = useKeys();
  const createBinding = useCreateBinding();
  const deleteBinding = useDeleteBinding();
  const updateApiKey = useUpdateApiKey();
  const [adding, setAdding] = useState(false);
  const [selectedServiceId, setSelectedServiceId] = useState("");
  const [deleteTarget, setDeleteTarget] = useState<AgentServiceBinding | null>(
    null,
  );

  // Filter services to those owned by the same owner as the API key.
  // Personal API key -> personal services only. Org-owned API key ->
  // services owned by the same org only. Anything else would be rejected
  // by `agent_binding_service::create_binding` because the cross-owner
  // check fails server-side.
  const services = (allKeys ?? []).filter(
    (s) => s.api_key_id && sameOwner(s.credential_source, apiKeySource),
  );

  // Already bound service IDs (to exclude from the dropdown)
  const boundServiceIds = new Set(
    (bindings ?? []).map((b) => b.user_service_id),
  );
  const availableServices = services.filter(
    (s) => !boundServiceIds.has(s.id),
  );

  function handleCreate() {
    if (!selectedServiceId) return;
    const service = services.find((s) => s.id === selectedServiceId);
    if (!service?.api_key_id) {
      toast.error("Selected service has no credential configured");
      return;
    }
    createBinding.mutate(
      {
        keyId,
        user_service_id: selectedServiceId,
        user_api_key_id: service.api_key_id,
      },
      {
        onSuccess: () => {
          toast.success("Service binding created");
          setAdding(false);
          setSelectedServiceId("");
        },
        onError: (err) => {
          const message =
            err instanceof ApiError
              ? err.message
              : "Failed to create binding";
          toast.error(message);
        },
      },
    );
  }

  function handleDelete() {
    if (!deleteTarget) return;
    deleteBinding.mutate(
      { keyId, bindingId: deleteTarget.id },
      {
        onSuccess: () => {
          toast.success("Binding removed");
          setDeleteTarget(null);
        },
        onError: (err) => {
          const message =
            err instanceof ApiError
              ? err.message
              : "Failed to remove binding";
          toast.error(message);
        },
      },
    );
  }

  return (
    <Card className="md:col-span-2">
      <CardHeader className="pb-3">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Link2 className="h-4 w-4 text-primary" />
            <CardTitle className="text-[15px]">Service Bindings</CardTitle>
          </div>
          <button
            type="button"
            onClick={() => setAdding(true)}
            disabled={adding || availableServices.length === 0}
            className="flex h-8 items-center gap-2 rounded-lg border border-white/[0.08] px-2.5 text-[12px] text-text-tertiary transition-all duration-300 hover:border-white/[0.15] hover:text-muted-foreground disabled:pointer-events-none disabled:opacity-40"
          >
            <span className="flex h-[18px] w-[18px] items-center justify-center rounded-[5px] border border-white/[0.08] bg-white/[0.04]">
              <Plus className="h-2.5 w-2.5" />
            </span>
            Add Service
          </button>
        </div>
        <CardDescription>
          {allowAllServices
            ? "This agent can access all services. Add bindings to override credentials for specific services."
            : "This agent can only access services listed below."}
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="flex items-center justify-between rounded-lg border border-border p-3">
          <div className="space-y-0.5">
            <Label htmlFor="allow-all-toggle" className="text-[12px] font-medium">
              Allow all services
            </Label>
            <p className="text-xs text-muted-foreground">
              {allowAllServices
                ? "Agent uses default credentials; bindings are overrides"
                : "Agent can only access services with bindings below"}
            </p>
          </div>
          <Switch
            id="allow-all-toggle"
            checked={allowAllServices}
            disabled={updateApiKey.isPending}
            onCheckedChange={(checked) => {
              // When restricting, seed allowed_service_ids from current bindings
              // so the agent doesn't lose access to already-bound services.
              const boundIds = checked
                ? undefined
                : (bindings ?? []).map((b) => b.user_service_id);
              updateApiKey.mutate(
                {
                  keyId,
                  allow_all_services: checked,
                  allowed_service_ids: boundIds,
                },
                {
                  onSuccess: () =>
                    checked
                      ? toast.success("Agent can now access all services")
                      : toast.warning("Agent restricted to bound services only"),
                  onError: (err) =>
                    toast.error(
                      err instanceof ApiError
                        ? err.message
                        : "Failed to update",
                    ),
                },
              );
            }}
          />
        </div>
        {adding && (
          <div className="rounded-lg border border-border p-3 space-y-3">
            <div className="space-y-1.5">
              <Label className="text-xs">AI Service</Label>
              <Select
                value={selectedServiceId}
                onValueChange={setSelectedServiceId}
              >
                <SelectTrigger>
                  <SelectValue placeholder="Select a service" />
                </SelectTrigger>
                <SelectContent>
                  {availableServices.map((s) => (
                    <SelectItem key={s.id} value={s.id}>
                      {s.label}
                      {s.slug !== s.label ? ` (${s.slug})` : ""}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="flex justify-end gap-2">
              <Button
                variant="outline"
                onClick={() => {
                  setAdding(false);
                  setSelectedServiceId("");
                }}
              >
                Cancel
              </Button>
              <Button
                variant="primary"
                onClick={handleCreate}
                disabled={createBinding.isPending || !selectedServiceId}
              >
                Add
              </Button>
            </div>
          </div>
        )}

        {isLoading ? (
          <div className="space-y-2">
            <Skeleton className="h-8 w-full" />
            <Skeleton className="h-8 w-full" />
          </div>
        ) : bindings && bindings.length > 0 ? (
          <div className="space-y-2">
            {bindings.map((b) => (
              <div
                key={b.id}
                className={
                  b.is_invalid
                    ? "flex items-center justify-between rounded-lg border border-destructive/40 bg-destructive/5 p-3"
                    : "flex items-center justify-between rounded-lg border border-border p-3"
                }
              >
                <div className="space-y-0.5">
                  <p className="text-[12px] font-medium flex items-center gap-1.5">
                    {b.is_invalid && (
                      <AlertTriangle
                        className="h-3.5 w-3.5 text-destructive shrink-0"
                        aria-hidden="true"
                      />
                    )}
                    <span>
                      {b.service_label}
                      {b.service_slug !== b.service_label && (
                        <span className="text-muted-foreground">
                          {" "}({b.service_slug})
                        </span>
                      )}
                    </span>
                  </p>
                  {b.is_invalid && (
                    <p className="text-xs text-destructive">
                      {invalidReasonLabel(b.invalid_reason)} Remove this
                      orphan binding to clean up.
                    </p>
                  )}
                </div>
                <Button
                  size="icon"
                  variant="ghost"
                  className="h-7 w-7 text-destructive hover:text-destructive"
                  onClick={() => setDeleteTarget(b)}
                >
                  <Trash2 className="h-3.5 w-3.5 text-destructive" />
                </Button>
              </div>
            ))}
          </div>
        ) : (
          <div className="flex flex-col items-center justify-center gap-1 py-8 text-center">
            <CrystalLatticeIcon className="h-48 w-48 text-muted-foreground/30" />
            <p className="text-xs text-muted-foreground/30">
              No credential overrides. This agent uses default credentials for
              all services.
            </p>
          </div>
        )}
      </CardContent>

      <Dialog
        open={deleteTarget !== null}
        onOpenChange={(open) => {
          if (!open) setDeleteTarget(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Remove Binding</DialogTitle>
            <DialogDescription>
              Remove &quot;{deleteTarget?.service_label}&quot; from this
              agent's service bindings?
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteTarget(null)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={handleDelete}
              disabled={deleteBinding.isPending}
            >
              Remove
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </Card>
  );
}
