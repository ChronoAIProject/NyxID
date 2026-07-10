import { useState } from "react";
import {
  useAdminOAuthClients,
  useBrokerSettings,
  useUpdateAdminOAuthClient,
  useUpdateBrokerSettings,
} from "@/hooks/use-admin-oauth-clients";
import { ApiError } from "@/lib/api-client";
import { formatDate } from "@/lib/utils";
import { canAdminWrite } from "@/types/api";
import type {
  AdminOAuthClient,
  BrokerSettingsResponse,
  BrokerPolicyField,
  UpdateAdminOAuthClientRequest,
  UpdateBrokerSettingsRequest,
} from "@/types/admin";
import { useAuthStore } from "@/stores/auth-store";
import { PageHeader } from "@/components/shared/page-header";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Switch } from "@/components/ui/switch";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { AlertTriangle, KeyRound, RotateCcw } from "lucide-react";
import { toast } from "sonner";

type ClientAction = {
  readonly client: AdminOAuthClient;
  readonly data: UpdateAdminOAuthClientRequest;
  readonly title: string;
  readonly description: string;
  readonly confirmLabel: string;
};

type BrokerSettingKey = keyof UpdateBrokerSettingsRequest;

type SettingAction = {
  readonly field: BrokerSettingKey;
  readonly value: boolean | null;
  readonly title: string;
  readonly description: string;
  readonly confirmLabel: string;
};

const SETTING_LABELS: Record<BrokerSettingKey, string> = {
  broker_require_sender_constraint: "Require sender constraint",
  broker_require_admin_capability: "Require admin broker capability",
};

const BROKER_BINDING_SCOPE = "urn:nyxid:scope:broker_binding";

export function AdminOAuthClientsPage() {
  const currentUser = useAuthStore((s) => s.user);
  const canWrite = canAdminWrite(currentUser);
  const [pendingClientAction, setPendingClientAction] =
    useState<ClientAction | null>(null);
  const [pendingSettingAction, setPendingSettingAction] =
    useState<SettingAction | null>(null);

  const { data, isLoading, error } = useAdminOAuthClients();
  const {
    data: brokerSettings,
    isLoading: settingsLoading,
    error: settingsError,
  } = useBrokerSettings(canWrite);
  const updateClient = useUpdateAdminOAuthClient();
  const updateSettings = useUpdateBrokerSettings();

  const clients = data?.clients ?? [];

  function stageBrokerCapability(client: AdminOAuthClient, enabled: boolean) {
    const removingLegacyScope =
      !enabled &&
      client.allowed_scopes.split(/\s+/).includes(BROKER_BINDING_SCOPE);
    const data: UpdateAdminOAuthClientRequest = enabled
      ? { broker_capability_enabled: true }
      : {
          broker_capability_enabled: false,
          allowed_scopes: removeBrokerBindingScope(client.allowed_scopes),
        };

    setPendingClientAction({
      client,
      data,
      title: enabled ? "Enable broker capability" : "Disable broker capability",
      description: enabled
        ? "This grants this app durable act-as-user broker capability. Only enable it for a reviewed OAuth client."
        : removingLegacyScope
          ? "This clears the admin broker flag and removes the legacy broker-binding scope so this app no longer remains broker-capable through scope-triggered compatibility."
          : "This removes durable act-as-user broker capability from this OAuth client.",
      confirmLabel: enabled ? "Enable capability" : "Disable capability",
    });
  }

  function stageActive(client: AdminOAuthClient, isActive: boolean) {
    setPendingClientAction({
      client,
      data: { is_active: isActive },
      title: isActive ? "Activate OAuth client" : "Deactivate OAuth client",
      description: isActive
        ? "This allows the OAuth client to participate in new authorization and token flows."
        : "This deactivates the OAuth client and clears its existing consents, refresh tokens, and pending authorization codes.",
      confirmLabel: isActive ? "Activate client" : "Deactivate client",
    });
  }

  async function confirmClientAction() {
    if (!pendingClientAction) return;
    try {
      await updateClient.mutateAsync({
        clientId: pendingClientAction.client.id,
        data: pendingClientAction.data,
      });
      toast.success("OAuth client updated");
      setPendingClientAction(null);
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to update OAuth client",
      );
    }
  }

  function stageSetting(field: BrokerSettingKey, value: boolean | null) {
    const label = SETTING_LABELS[field];
    const isReset = value === null;
    const enabled = value === true;
    const description =
      field === "broker_require_sender_constraint"
        ? settingSenderConstraintCopy(value)
        : settingAdminCapabilityCopy(value);

    setPendingSettingAction({
      field,
      value,
      title: isReset
        ? `Reset ${label}`
        : `${enabled ? "Enable" : "Disable"} ${label}`,
      description,
      confirmLabel: isReset
        ? "Reset to env default"
        : enabled
          ? "Enable setting"
          : "Disable setting",
    });
  }

  async function confirmSettingAction() {
    if (!pendingSettingAction) return;
    try {
      await updateSettings.mutateAsync({
        [pendingSettingAction.field]: pendingSettingAction.value,
      });
      toast.success("Broker settings updated");
      setPendingSettingAction(null);
    } catch (err) {
      toast.error(
        err instanceof ApiError
          ? err.message
          : "Failed to update broker settings",
      );
    }
  }

  return (
    <div className="space-y-8">
      <PageHeader
        title="OAuth Clients"
        description="Manage platform OAuth clients, including dynamic registrations used by broker-capable apps."
      />

      {canWrite && (
        <BrokerSettingsSection
          settings={brokerSettings}
          isLoading={settingsLoading}
          error={settingsError}
          onStage={stageSetting}
          disabled={updateSettings.isPending}
        />
      )}

      {isLoading ? (
        <div className="space-y-2">
          {Array.from({ length: 5 }).map((_, i) => (
            <Skeleton
              key={`oauth-client-skel-${String(i)}`}
              className="h-12 w-full"
            />
          ))}
        </div>
      ) : error ? (
        <div className="flex flex-col items-center justify-center gap-2 rounded-lg border border-destructive/30 bg-destructive/5 px-4 py-12 text-center">
          <AlertTriangle className="h-8 w-8 text-destructive" />
          <p className="text-sm font-medium text-destructive">
            Failed to load OAuth clients
          </p>
        </div>
      ) : clients.length === 0 ? (
        <div className="flex flex-col items-center justify-center gap-3 rounded-lg border border-border/50 bg-card px-4 py-14 text-center">
          <KeyRound className="h-10 w-10 text-muted-foreground" />
          <div>
            <p className="text-sm font-medium text-foreground">
              No OAuth clients
            </p>
            <p className="text-xs text-muted-foreground">
              Registered clients will appear here.
            </p>
          </div>
        </div>
      ) : (
        <div className="overflow-x-auto rounded-xl border border-border/50 bg-card">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Client</TableHead>
                <TableHead>Type</TableHead>
                <TableHead>Created By</TableHead>
                <TableHead>Broker</TableHead>
                <TableHead>Status</TableHead>
                <TableHead>Allowed Scopes</TableHead>
                <TableHead>Created</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {clients.map((client) => (
                <TableRow key={client.id}>
                  <TableCell className="min-w-[220px]">
                    <div className="space-y-1">
                      <p className="text-sm font-medium text-foreground">
                        {client.client_name}
                      </p>
                      <p className="font-mono text-[11px] text-muted-foreground">
                        {client.id}
                      </p>
                    </div>
                  </TableCell>
                  <TableCell>
                    <Badge variant="secondary">{client.client_type}</Badge>
                  </TableCell>
                  <TableCell className="min-w-[170px]">
                    <span className="font-mono text-[11px] text-muted-foreground">
                      {client.created_by ?? "ownerless"}
                    </span>
                  </TableCell>
                  <TableCell>
                    {canWrite ? (
                      <div className="flex items-center gap-2">
                        <Switch
                          aria-label={`Toggle broker capability for ${client.client_name}`}
                          checked={client.broker_capability_effective}
                          disabled={updateClient.isPending}
                          onCheckedChange={(checked) =>
                            stageBrokerCapability(client, checked)
                          }
                        />
                        <BrokerSourceBadge
                          source={client.broker_capability_source}
                        />
                      </div>
                    ) : (
                      <div className="flex items-center gap-2">
                        <BoolBadge value={client.broker_capability_effective} />
                        <BrokerSourceBadge
                          source={client.broker_capability_source}
                        />
                      </div>
                    )}
                  </TableCell>
                  <TableCell>
                    {canWrite ? (
                      <Switch
                        aria-label={`Toggle active status for ${client.client_name}`}
                        checked={client.is_active}
                        disabled={updateClient.isPending}
                        onCheckedChange={(checked) =>
                          stageActive(client, checked)
                        }
                      />
                    ) : (
                      <BoolBadge value={client.is_active} />
                    )}
                  </TableCell>
                  <TableCell className="min-w-[260px] max-w-[360px]">
                    <ScopeList scopes={client.allowed_scopes} />
                  </TableCell>
                  <TableCell className="whitespace-nowrap text-sm text-muted-foreground">
                    {formatDate(client.created_at)}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      )}

      <Dialog
        open={canWrite && pendingClientAction !== null}
        onOpenChange={(open) => {
          if (!open) setPendingClientAction(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{pendingClientAction?.title}</DialogTitle>
            <DialogDescription>
              {pendingClientAction?.description}
            </DialogDescription>
          </DialogHeader>
          {pendingClientAction && (
            <div className="rounded-lg bg-muted/50 p-3">
              <p className="text-sm font-medium">
                {pendingClientAction.client.client_name}
              </p>
              <p className="mt-1 font-mono text-[11px] text-muted-foreground">
                {pendingClientAction.client.id}
              </p>
            </div>
          )}
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => setPendingClientAction(null)}
            >
              Cancel
            </Button>
            <Button
              type="button"
              onClick={() => void confirmClientAction()}
              disabled={updateClient.isPending}
            >
              {pendingClientAction?.confirmLabel ?? "Confirm"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog
        open={canWrite && pendingSettingAction !== null}
        onOpenChange={(open) => {
          if (!open) setPendingSettingAction(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{pendingSettingAction?.title}</DialogTitle>
            <DialogDescription>
              {pendingSettingAction?.description}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => setPendingSettingAction(null)}
            >
              Cancel
            </Button>
            <Button
              type="button"
              onClick={() => void confirmSettingAction()}
              disabled={updateSettings.isPending}
            >
              {pendingSettingAction?.confirmLabel ?? "Confirm"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

function BrokerSettingsSection({
  settings,
  isLoading,
  error,
  onStage,
  disabled,
}: {
  readonly settings: BrokerSettingsResponse | undefined;
  readonly isLoading: boolean;
  readonly error: unknown;
  readonly onStage: (field: BrokerSettingKey, value: boolean | null) => void;
  readonly disabled: boolean;
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>Broker Rollout Policy</CardTitle>
        <CardDescription>
          Runtime overrides are applied immediately and fall back to env
          defaults when reset.
        </CardDescription>
      </CardHeader>
      <CardContent>
        {isLoading ? (
          <div className="space-y-2">
            <Skeleton className="h-14 w-full" />
            <Skeleton className="h-14 w-full" />
          </div>
        ) : error ? (
          <div className="flex items-center gap-2 rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive">
            <AlertTriangle className="h-4 w-4" />
            Failed to load broker settings
          </div>
        ) : settings ? (
          <div className="divide-y divide-border/60">
            <BrokerPolicyRow
              label="Require sender constraint"
              setting={settings.broker_require_sender_constraint}
              disabled={disabled}
              onToggle={(checked) =>
                onStage("broker_require_sender_constraint", checked)
              }
              onReset={() => onStage("broker_require_sender_constraint", null)}
            />
            <BrokerPolicyRow
              label="Require admin broker capability"
              setting={settings.broker_require_admin_capability}
              disabled={disabled}
              onToggle={(checked) =>
                onStage("broker_require_admin_capability", checked)
              }
              onReset={() => onStage("broker_require_admin_capability", null)}
            />
          </div>
        ) : null}
      </CardContent>
    </Card>
  );
}

function BrokerPolicyRow({
  label,
  setting,
  disabled,
  onToggle,
  onReset,
}: {
  readonly label: string;
  readonly setting: BrokerPolicyField;
  readonly disabled: boolean;
  readonly onToggle: (checked: boolean) => void;
  readonly onReset: () => void;
}) {
  const overridden = setting.source === "override";
  return (
    <div className="flex flex-col gap-3 py-4 first:pt-0 last:pb-0 sm:flex-row sm:items-center sm:justify-between">
      <div className="min-w-0 space-y-1">
        <div className="flex flex-wrap items-center gap-2">
          <p className="text-sm font-medium text-foreground">{label}</p>
          <Badge variant={overridden ? "default" : "secondary"}>
            {overridden ? "Overridden" : "Env default"}
          </Badge>
          <BoolBadge value={setting.effective} />
        </div>
        <p className="text-xs text-muted-foreground">
          Env default: {setting.env_default ? "enabled" : "disabled"}
          {overridden
            ? ` · Override: ${setting.override ? "enabled" : "disabled"}`
            : ""}
        </p>
      </div>
      <div className="flex items-center gap-2">
        {overridden && (
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={onReset}
            disabled={disabled}
          >
            <RotateCcw className="mr-2 h-3.5 w-3.5" />
            Reset
          </Button>
        )}
        <Switch
          aria-label={`Toggle ${label}`}
          checked={setting.effective}
          disabled={disabled}
          onCheckedChange={onToggle}
        />
      </div>
    </div>
  );
}

function BrokerSourceBadge({
  source,
}: {
  readonly source: AdminOAuthClient["broker_capability_source"];
}) {
  if (source === "none") return null;
  return (
    <Badge variant="secondary" className="whitespace-nowrap">
      {source === "flag" ? "Flag" : "Scope"}
    </Badge>
  );
}

function BoolBadge({ value }: { readonly value: boolean }) {
  return (
    <Badge variant={value ? "default" : "secondary"}>
      {value ? "Enabled" : "Disabled"}
    </Badge>
  );
}

function ScopeList({ scopes }: { readonly scopes: string }) {
  const items = scopes.split(/\s+/).filter(Boolean);
  if (items.length === 0) {
    return <span className="text-sm text-muted-foreground">None</span>;
  }
  return (
    <div className="flex flex-wrap gap-1.5">
      {items.map((scope) => (
        <Badge
          key={scope}
          variant="secondary"
          className="font-mono text-[10px]"
        >
          {scope}
        </Badge>
      ))}
    </div>
  );
}

function removeBrokerBindingScope(scopes: string): string[] {
  const remaining = scopes
    .split(/\s+/)
    .filter((scope) => scope && scope !== BROKER_BINDING_SCOPE);
  return remaining.length > 0 ? remaining : ["openid"];
}

function settingSenderConstraintCopy(value: boolean | null): string {
  if (value === null) {
    return "This clears the runtime override and returns sender-constraint enforcement to the env default.";
  }
  if (value) {
    return "This requires broker bindings to be DPoP or mTLS sender-constrained. Unpinned broker bindings will be rejected.";
  }
  return "This disables the runtime sender-constraint requirement and allows the env default to be overridden off.";
}

function settingAdminCapabilityCopy(value: boolean | null): string {
  if (value === null) {
    return "This clears the runtime override and returns admin-capability enforcement to the env default.";
  }
  if (value) {
    return "This requires platform-admin provisioning before an OAuth client can receive durable act-as-user broker capability.";
  }
  return "This disables the runtime admin-capability requirement and allows self-service broker-scope clients under this override.";
}
