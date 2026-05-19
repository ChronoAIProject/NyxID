import { useState } from "react";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  useNotificationSettings,
  useUpdateNotificationSettings,
  useTelegramLink,
  useTelegramDisconnect,
  usePushDevices,
  useRemoveDevice,
  useServiceApprovalConfigs,
  useSetServiceApprovalConfig,
  useDeleteServiceApprovalConfig,
} from "@/hooks/use-approvals";
import { useUserServices } from "@/hooks/use-user-services";
import {
  updateNotificationSettingsSchema,
  type UpdateNotificationSettingsFormData,
} from "@/schemas/approvals";
import { ApiError } from "@/lib/api-client";
import { ErrorBanner } from "@/components/shared/error-banner";
import { PageHeader } from "@/components/shared/page-header";
import { Skeleton } from "@/components/ui/skeleton";
import { Button, ButtonIcon } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Form,
  FormControl,
  FormDescription,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@/components/ui/form";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  RotateCcw,
  Smartphone,
  Trash2,
  Unlink,
} from "lucide-react";
import { toast } from "sonner";
import { ApprovalSetupWizard } from "@/components/shared/approval-setup-wizard";
import { AddCtaButton } from "@/components/shared/add-cta-button";

export function NotificationSettingsPage() {
  const { data: rawSettings, isLoading, error, refetch } = useNotificationSettings();
  const updateMutation = useUpdateNotificationSettings();
  const telegramLinkMutation = useTelegramLink();
  const telegramDisconnectMutation = useTelegramDisconnect();

  const { data: rawPushDevices, isLoading: isPushDevicesLoading } =
    usePushDevices();
  const removeDeviceMutation = useRemoveDevice();

  // DEV: ?mock=disconnected forces both channels to appear unconnected
  const isMockDisconnected = new URLSearchParams(window.location.search).get("mock") === "disconnected";
  const settings = isMockDisconnected && rawSettings
    ? { ...rawSettings, telegram_connected: false, telegram_username: null }
    : rawSettings;
  const pushDevices = isMockDisconnected
    ? { devices: [] }
    : rawPushDevices;

  const { data: userServices } = useUserServices();
  const {
    data: serviceConfigs,
    isLoading: isServiceConfigsLoading,
    error: serviceConfigsError,
    refetch: refetchServiceConfigs,
  } = useServiceApprovalConfigs();
  const setConfigMutation = useSetServiceApprovalConfig();
  const deleteConfigMutation = useDeleteServiceApprovalConfig();

  const [linkDialogOpen, setLinkDialogOpen] = useState(false);
  const [disconnectDialogOpen, setDisconnectDialogOpen] = useState(false);
  const [removeDeviceId, setRemoveDeviceId] = useState<string | null>(null);
  const [addServiceDialogOpen, setAddServiceDialogOpen] = useState(false);
  const [selectedServiceId, setSelectedServiceId] = useState("");
  const [selectedApprovalRequired, setSelectedApprovalRequired] =
    useState(true);
  const [selectedApprovalMode, setSelectedApprovalMode] = useState<
    "per_request" | "grant"
  >("per_request");

  const linkData = telegramLinkMutation.data;

  const form = useForm<UpdateNotificationSettingsFormData>({
    resolver: zodResolver(updateNotificationSettingsSchema),
    values: settings
      ? {
          telegram_enabled: settings.telegram_enabled,
          push_enabled: settings.push_enabled,
          approval_required: settings.approval_required,
          approval_timeout_secs: settings.approval_timeout_secs,
          grant_expiry_days: settings.grant_expiry_days,
        }
      : undefined,
  });

  // Identifier set used to dedupe the picker. A single config may be
  // keyed by either:
  //   (a) a `UserService.id` (custom services, or explicitly targeted),
  //   (b) a catalog `DownstreamService.id` that covers *all* of the
  //       user's services sharing that `catalog_service_id`.
  // Collect both so the picker hides every service already covered — not
  // just the `user_service_id` the backend happened to return for case
  // (b) (which is the most-recently-created sibling). Without this, an
  // org/user with two OpenAI user services would still see the other one
  // in the Add dialog and "adding" it would silently overwrite the same
  // catalog-wide policy.
  const configuredIds = new Set<string>();
  for (const c of serviceConfigs?.configs ?? []) {
    configuredIds.add(c.service_id);
    if (c.user_service_id) configuredIds.add(c.user_service_id);
  }

  // `(org_id, service_id)` pairs where a specific org dominates the
  // approval policy for a catalog service the caller inherits from it.
  // `resolve_org_aware_approval` picks that org's policy before falling
  // back to the actor's personal config, so personal overrides are
  // silently ignored *for calls routed through that org*. We index by
  // `org_id|service_id` so the picker only hides the org-specific
  // entry; if the same catalog service is also inherited from a
  // different org without a dominant policy, that entry stays
  // selectable. Omitted on the org-scoped list — default to empty.
  const dominantOrgPolicyKeys = new Set<string>(
    (serviceConfigs?.dominant_org_policies ?? []).map(
      (p) => `${p.org_id}|${p.service_id}`,
    ),
  );

  // Picker entries for the Add-Override dialog. Two kinds of services
  // are pickable:
  //
  //  1. **Personal** — the actor owns the UserService outright. We use
  //     the `UserService.id` as the mutation key so the backend maps it
  //     to the effective storage key (catalog id when catalog-backed,
  //     user service id for custom services — see ChronoAIProject/NyxID#165).
  //
  //  2. **Org-inherited with a catalog id** — the UserService belongs to
  //     an org the actor has proxy access to (member/admin), and the
  //     org has no dominant per-service policy of its own. In that case
  //     `resolve_org_aware_approval` falls back to the actor's personal
  //     per-service config keyed by the catalog id, so the user *can*
  //     author a personal override that affects their org-routed proxy
  //     calls. We use `catalog_service_id` as the mutation key (the
  //     legacy path accepts catalog ids) and label the entry with the
  //     org name so the user knows the scope.
  //
  // Viewer-role entries carry `allowed: false` and are excluded — they
  // can't proxy, so an approval policy would have nothing to gate.
  // Custom (no-catalog) org services are also excluded: a personal
  // policy can only key on the catalog id for org-shared resources,
  // and there isn't one.
  type PickerEntry = {
    readonly id: string;
    readonly label: string;
  };
  const selectableUserServices: readonly PickerEntry[] =
    isServiceConfigsLoading || serviceConfigsError
      ? []
      : (() => {
          const out: PickerEntry[] = [];
          const seen = new Set<string>();
          for (const s of userServices ?? []) {
            if (!s.is_active) continue;
            const src = s.credential_source;
            let mutationId: string | null = null;
            let label = s.slug;
            if (src.type === "personal") {
              mutationId = s.id;
            } else if (src.type === "org" && src.allowed && s.catalog_service_id) {
              mutationId = s.catalog_service_id;
              label = `${s.slug} (via ${src.org_name})`;
            }
            if (!mutationId) continue;
            if (configuredIds.has(mutationId)) continue;
            if (configuredIds.has(s.id)) continue;
            if (s.catalog_service_id && configuredIds.has(s.catalog_service_id))
              continue;
            // Skip org-inherited entries whose *specific* org already
            // has a dominant policy for this catalog — a personal
            // override would be ignored at proxy time. Scoped per
            // (org_id, service_id) so another org inheriting the same
            // catalog without its own policy stays selectable.
            if (
              src.type === "org" &&
              s.catalog_service_id &&
              dominantOrgPolicyKeys.has(
                `${src.org_id}|${s.catalog_service_id}`,
              )
            )
              continue;
            if (seen.has(mutationId)) continue;
            seen.add(mutationId);
            out.push({ id: mutationId, label });
          }
          return out;
        })();

  async function handleSave(data: UpdateNotificationSettingsFormData) {
    try {
      await updateMutation.mutateAsync(data);
      toast.success("Notification settings updated");
    } catch (err) {
      if (err instanceof ApiError) {
        form.setError("root", { message: err.message });
      } else {
        toast.error("Failed to update settings");
      }
    }
  }

  async function handleLinkTelegram() {
    try {
      await telegramLinkMutation.mutateAsync();
      setLinkDialogOpen(true);
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to generate link code",
      );
    }
  }

  async function handleDisconnect() {
    try {
      await telegramDisconnectMutation.mutateAsync();
      toast.success("Telegram disconnected");
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to disconnect Telegram",
      );
    } finally {
      setDisconnectDialogOpen(false);
    }
  }

  async function handleRemoveDevice() {
    if (!removeDeviceId) return;

    try {
      await removeDeviceMutation.mutateAsync(removeDeviceId);
      toast.success("Device removed");
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to remove device",
      );
    } finally {
      setRemoveDeviceId(null);
    }
  }

  async function handleAddServiceConfig() {
    if (!selectedServiceId) return;

    try {
      await setConfigMutation.mutateAsync({
        serviceId: selectedServiceId,
        approvalRequired: selectedApprovalRequired,
        approvalMode: selectedApprovalMode,
      });
      toast.success("Per-service approval override added");
      setAddServiceDialogOpen(false);
      setSelectedServiceId("");
      setSelectedApprovalRequired(true);
      setSelectedApprovalMode("per_request");
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to add service config",
      );
    }
  }

  async function handleToggleServiceConfig(
    serviceId: string,
    approvalRequired: boolean,
  ) {
    try {
      // `serviceId` here is the mutation key — `user_service_id` when
      // known, else the raw stored `service_id`. Match against either so
      // catalog-keyed rows resolve too (their `service_id` is a catalog
      // id, not a UserService id).
      const existingConfig = serviceConfigs?.configs.find(
        (c) =>
          c.user_service_id === serviceId || c.service_id === serviceId,
      );
      await setConfigMutation.mutateAsync({
        serviceId,
        approvalRequired,
        approvalMode: existingConfig?.approval_mode,
      });
    } catch (err) {
      toast.error(
        err instanceof ApiError
          ? err.message
          : "Failed to update service config",
      );
    }
  }

  async function handleChangeApprovalMode(
    serviceId: string,
    approvalRequired: boolean,
    approvalMode: "per_request" | "grant",
  ) {
    try {
      await setConfigMutation.mutateAsync({
        serviceId,
        approvalRequired,
        approvalMode,
      });
    } catch (err) {
      toast.error(
        err instanceof ApiError
          ? err.message
          : "Failed to update approval mode",
      );
    }
  }

  async function handleDeleteServiceConfig(serviceId: string) {
    try {
      await deleteConfigMutation.mutateAsync({ serviceId });
      toast.success("Per-service override removed");
    } catch (err) {
      toast.error(
        err instanceof ApiError
          ? err.message
          : "Failed to remove service config",
      );
    }
  }

  return (
    <div className="space-y-8">
      <PageHeader
        title="Notification Settings"
        description="Configure how you receive approval notifications."
      />

      {isLoading ? (
        <div className="space-y-4">
          <Skeleton className="h-48 w-full" />
          <Skeleton className="h-64 w-full" />
        </div>
      ) : error ? (
        <ErrorBanner message="Failed to load notification settings. Please try again." onRetry={refetch} />
      ) : (
        <div className="space-y-6">
          {/* Setup Wizard */}
          <ApprovalSetupWizard
            hasChannel={
              Boolean(settings?.telegram_connected) ||
              Boolean(pushDevices?.devices.length)
            }
            channelEnabled={
              Boolean(settings?.telegram_enabled) ||
              Boolean(settings?.push_enabled)
            }
            approvalEnabled={Boolean(settings?.approval_required)}
          />

          {/* Telegram Connection Card */}
          <Card>
            <CardHeader>
              <CardTitle>Telegram Connection</CardTitle>
              <CardDescription>
                Connect your Telegram account to receive approval notifications.
              </CardDescription>
            </CardHeader>
            <CardContent>
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-3">
                  {settings?.telegram_connected ? (
                    <>
                      <Badge variant="success">Connected</Badge>
                      {settings.telegram_username && (
                        <span className="text-[12px] text-muted-foreground">
                          {settings.telegram_username}
                        </span>
                      )}
                    </>
                  ) : (
                    <Badge variant="secondary">Not connected</Badge>
                  )}
                </div>
                {settings?.telegram_connected ? (
                  <Button
                    variant="outline"
                    className="text-text-tertiary hover:text-muted-foreground"
                    onClick={() => setDisconnectDialogOpen(true)}
                  >
                    <ButtonIcon><Unlink className="h-3 w-3" /></ButtonIcon>
                    Disconnect
                  </Button>
                ) : (
                  <Button
                    variant="outline"
                    className="text-text-tertiary hover:text-muted-foreground"
                    onClick={() => void handleLinkTelegram()}
                    isLoading={telegramLinkMutation.isPending}
                  >
                    <ButtonIcon>
                      <svg className="h-3 w-3" viewBox="0 0 24 24" fill="currentColor">
                        <path d="M11.944 0A12 12 0 0 0 0 12a12 12 0 0 0 12 12 12 12 0 0 0 12-12A12 12 0 0 0 12 0a12 12 0 0 0-.056 0zm4.962 7.224c.1-.002.321.023.465.14a.506.506 0 0 1 .171.325c.016.093.036.306.02.472-.18 1.898-.962 6.502-1.36 8.627-.168.9-.499 1.201-.82 1.23-.696.065-1.225-.46-1.9-.902-1.056-.693-1.653-1.124-2.678-1.8-1.185-.78-.417-1.21.258-1.91.177-.184 3.247-2.977 3.307-3.23.007-.032.014-.15-.056-.212s-.174-.041-.249-.024c-.106.024-1.793 1.14-5.061 3.345-.48.33-.913.49-1.302.48-.428-.008-1.252-.241-1.865-.44-.752-.245-1.349-.374-1.297-.789.027-.216.325-.437.893-.663 3.498-1.524 5.83-2.529 6.998-3.014 3.332-1.386 4.025-1.627 4.476-1.635z" />
                      </svg>
                    </ButtonIcon>
                    Connect Telegram
                  </Button>
                )}
              </div>
            </CardContent>
          </Card>

          {/* Push Devices Card */}
          <Card>
            <CardHeader>
              <CardTitle>Push Devices</CardTitle>
              <CardDescription>
                Mobile devices registered for push notifications. Devices are
                registered from the NyxID mobile app.
              </CardDescription>
            </CardHeader>
            <CardContent>
              {isPushDevicesLoading ? (
                <div className="space-y-3 py-2">
                  <Skeleton className="h-14 w-full" />
                  <Skeleton className="h-14 w-full" />
                </div>
              ) : !pushDevices?.devices.length ? (
                <div className="space-y-3">
                  <div className="rounded-lg bg-white/[0.03] px-4 py-3 text-[12px] text-muted-foreground">
                    No devices registered. Install the NyxID mobile app and sign
                    in to register a device.
                  </div>
                  <div className="flex justify-end">
                    <Button variant="primary" asChild>
                      <a href="https://nyxid.dev/app" target="_blank" rel="noopener noreferrer">
                        <ButtonIcon className="border-white/20 bg-white/10">
                          <Smartphone className="h-3 w-3" />
                        </ButtonIcon>
                        Get App
                      </a>
                    </Button>
                  </div>
                </div>
              ) : (
                <div className="space-y-3">
                  {pushDevices.devices.map((device) => (
                    <div
                      key={device.device_id}
                      className="flex items-center justify-between rounded-lg border border-border p-4"
                    >
                      <div className="flex items-center gap-3">
                        <Badge
                          variant={
                            device.platform === "apns" ? "secondary" : "secondary"
                          }
                        >
                          {device.platform === "apns" ? "iOS" : "Android"}
                        </Badge>
                        <div className="space-y-0.5">
                          <p className="text-[12px] font-medium">
                            {device.device_name ?? "Unknown device"}
                          </p>
                          <p className="text-xs text-muted-foreground">
                            Registered{" "}
                            {new Date(
                              device.registered_at,
                            ).toLocaleDateString()}
                            {device.last_used_at && (
                              <>
                                {" "}
                                &middot; Last used{" "}
                                {new Date(
                                  device.last_used_at,
                                ).toLocaleDateString()}
                              </>
                            )}
                          </p>
                        </div>
                      </div>
                      <Button
                        variant="ghost"
                        size="icon"
                        onClick={() => setRemoveDeviceId(device.device_id)}
                        title="Remove device"
                      >
                        <Trash2 className="h-4 w-4 text-destructive" />
                      </Button>
                    </div>
                  ))}
                </div>
              )}
            </CardContent>
          </Card>

          {/* Approval Preferences */}
          <Card>
            <CardHeader>
              <CardTitle>Approval Preferences</CardTitle>
              <CardDescription>
                Configure approval settings. When enabled, every request
                requires approval by default (per-request mode). You can
                opt specific services into time-based grants below.
              </CardDescription>
            </CardHeader>
            <CardContent>
              <Form {...form}>
                <form
                  onSubmit={form.handleSubmit((data) => void handleSave(data))}
                  className="space-y-6"
                >
                  {form.formState.errors.root && (
                    <div
                      role="alert"
                      className="rounded-lg bg-destructive/10 p-3 text-[12px] text-destructive"
                    >
                      {form.formState.errors.root.message}
                    </div>
                  )}

                  <FormField
                    control={form.control}
                    name="approval_required"
                    render={({ field }) => (
                      <FormItem className="!space-y-0 flex items-center justify-between rounded-lg border border-border p-4">
                        <div className="space-y-0.5">
                          <FormLabel>
                            Require Approval (Global Default)
                          </FormLabel>
                          <FormDescription>
                            When enabled, every proxy and LLM gateway request
                            requires a fresh approval (per-request mode by
                            default). Use per-service overrides below to switch
                            individual services to time-based grants or to
                            exempt them entirely.
                          </FormDescription>
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

                  <FormField
                    control={form.control}
                    name="telegram_enabled"
                    render={({ field }) => (
                      <FormItem className="!space-y-0 flex items-center justify-between rounded-lg border border-border p-4">
                        <div className="space-y-0.5">
                          <FormLabel>
                            Telegram Notifications
                          </FormLabel>
                          <FormDescription>
                            Send approval requests via Telegram.
                          </FormDescription>
                        </div>
                        <FormControl>
                          <Switch
                            checked={field.value}
                            onCheckedChange={field.onChange}
                            disabled={!settings?.telegram_connected}
                          />
                        </FormControl>
                      </FormItem>
                    )}
                  />

                  <FormField
                    control={form.control}
                    name="push_enabled"
                    render={({ field }) => (
                      <FormItem className="!space-y-0 flex items-center justify-between rounded-lg border border-border p-4">
                        <div className="space-y-0.5">
                          <FormLabel>
                            Push Notifications
                          </FormLabel>
                          <FormDescription>
                            Send approval requests to your registered mobile
                            devices.
                          </FormDescription>
                        </div>
                        <FormControl>
                          <Switch
                            checked={field.value}
                            onCheckedChange={field.onChange}
                            disabled={!pushDevices?.devices.length}
                          />
                        </FormControl>
                      </FormItem>
                    )}
                  />

                  <FormField
                    control={form.control}
                    name="approval_timeout_secs"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>Approval Timeout (seconds)</FormLabel>
                        <FormControl>
                          <Input
                            type="number"
                            min={10}
                            max={300}
                            value={String(field.value)}
                            onChange={(e) =>
                              field.onChange(Number(e.target.value))
                            }
                            onBlur={field.onBlur}
                            name={field.name}
                            ref={field.ref}
                          />
                        </FormControl>
                        <FormDescription>
                          How long to wait for a response before auto-rejecting
                          (10-300 seconds).
                        </FormDescription>
                        <FormMessage />
                      </FormItem>
                    )}
                  />

                  <FormField
                    control={form.control}
                    name="grant_expiry_days"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>Grant Expiry (days)</FormLabel>
                        <FormControl>
                          <Input
                            type="number"
                            min={1}
                            max={365}
                            value={String(field.value)}
                            onChange={(e) =>
                              field.onChange(Number(e.target.value))
                            }
                            onBlur={field.onBlur}
                            name={field.name}
                            ref={field.ref}
                          />
                        </FormControl>
                        <FormDescription>
                          How many days an approval grant lasts before
                          re-prompting (1-365 days). Only applies to services
                          configured with time-based grant mode.
                        </FormDescription>
                        <FormMessage />
                      </FormItem>
                    )}
                  />

                  <div className="flex justify-end">
                    <Button variant="primary" type="submit" isLoading={updateMutation.isPending} disabled={!form.formState.isDirty || updateMutation.isPending}>
                      Save Preferences
                    </Button>
                  </div>
                </form>
              </Form>
            </CardContent>
          </Card>

          {/* Per-Service Approval Overrides */}
          <Card>
            <CardHeader>
              <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
                <div className="space-y-1.5">
                  <CardTitle>Per-Service Approval Overrides</CardTitle>
                  <CardDescription>
                    Override the global approval setting for specific services.
                    Services without an override use per-request approval by
                    default. Use overrides to switch services to time-based
                    grants or to exempt them from approval.
                  </CardDescription>
                </div>
                <div className="shrink-0 self-start sm:self-center">
                  <AddCtaButton
                    label="Add Override"
                    onClick={() => setAddServiceDialogOpen(true)}
                    disabled={
                      isServiceConfigsLoading ||
                      Boolean(serviceConfigsError) ||
                      selectableUserServices.length === 0
                    }
                  />
                </div>
              </div>
            </CardHeader>
            <CardContent>
              {isServiceConfigsLoading ? (
                <div className="space-y-3 py-2">
                  <Skeleton className="h-16 w-full" />
                  <Skeleton className="h-16 w-full" />
                </div>
              ) : serviceConfigsError ? (
                <ErrorBanner message="Failed to load per-service overrides. Try refreshing the page." onRetry={refetchServiceConfigs} />
              ) : serviceConfigs?.configs.length === 0 ? (
                <div className="rounded-lg bg-white/[0.03] px-4 py-3 text-[12px] text-muted-foreground">
                  No per-service overrides configured. All services use the
                  global default.
                </div>
              ) : (
                <div className="space-y-3">
                  {serviceConfigs?.configs.map((config) => {
                    // Prefer the UserService id for API calls so the
                    // backend resolves the same row the user sees on the
                    // Keys page. Falls back to the raw service_id for
                    // legacy rows without a matching active UserService.
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
                            {config.user_service_slug
                              ? `${config.user_service_slug} — `
                              : ""}
                            {config.approval_required
                              ? "Approval required"
                              : "Approval not required"}
                          </p>
                        </div>
                        <div className="flex items-center gap-3">
                          <Switch
                            checked={config.approval_required}
                            onCheckedChange={(checked) =>
                              void handleToggleServiceConfig(
                                mutationKey,
                                checked,
                              )
                            }
                          />
                          <Button
                            variant="ghost"
                            size="icon"
                            onClick={() =>
                              void handleDeleteServiceConfig(mutationKey)
                            }
                            title="Remove override (use global default)"
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
                              void handleChangeApprovalMode(
                                mutationKey,
                                config.approval_required,
                                value as "per_request" | "grant",
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
        </div>
      )}

      {/* Telegram Link Dialog */}
      <Dialog open={linkDialogOpen} onOpenChange={setLinkDialogOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Connect Telegram</DialogTitle>
            <DialogDescription>
              Send the following command to the NyxID bot on Telegram to link
              your account.
            </DialogDescription>
          </DialogHeader>
          {linkData && (
            <div className="space-y-4">
              <div className="rounded-lg bg-muted p-4 text-center">
                <p className="text-xs text-muted-foreground">
                  Send this to @{linkData.bot_username}
                </p>
                <code className="mt-2 block text-lg font-semibold">
                  /start {linkData.link_code}
                </code>
              </div>
              <p className="text-xs text-muted-foreground">
                This code expires in{" "}
                {String(Math.floor(linkData.expires_in_secs / 60))} minutes.
              </p>
            </div>
          )}
          <DialogFooter>
            <Button variant="outline" onClick={() => setLinkDialogOpen(false)}>
              Close
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Disconnect Confirmation */}
      <Dialog
        open={disconnectDialogOpen}
        onOpenChange={setDisconnectDialogOpen}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Disconnect Telegram</DialogTitle>
            <DialogDescription>
              Are you sure you want to disconnect your Telegram account? You
              will no longer receive approval notifications via Telegram.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setDisconnectDialogOpen(false)}
            >
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => void handleDisconnect()}
              isLoading={telegramDisconnectMutation.isPending}
            >
              Disconnect
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Remove Device Confirmation */}
      <Dialog
        open={removeDeviceId !== null}
        onOpenChange={(open) => {
          if (!open) setRemoveDeviceId(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Remove Device</DialogTitle>
            <DialogDescription>
              Are you sure you want to remove this device? It will no longer
              receive push notifications. You can re-register it from the mobile
              app.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setRemoveDeviceId(null)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => void handleRemoveDevice()}
              isLoading={removeDeviceMutation.isPending}
            >
              Remove Device
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Add Per-Service Override Dialog */}
      <Dialog
        open={addServiceDialogOpen}
        onOpenChange={setAddServiceDialogOpen}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Add Per-Service Override</DialogTitle>
            <DialogDescription>
              Choose a service and set whether approval is required for it,
              overriding the global default.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-4 py-4">
            <div className="space-y-2">
              <label className="text-[12px] font-medium" htmlFor="service-select">
                Service
              </label>
              <Select
                value={selectedServiceId}
                onValueChange={setSelectedServiceId}
              >
                <SelectTrigger id="service-select">
                  <SelectValue placeholder="Select a service" />
                </SelectTrigger>
                <SelectContent>
                  {selectableUserServices.map((s) => (
                    <SelectItem key={s.id} value={s.id}>
                      {s.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <p className="text-xs text-muted-foreground">
                Shows your configured AI services. To add a new one, go to
                the Keys page.
              </p>
            </div>
            <div className="flex items-center justify-between rounded-lg border border-border p-4">
              <div className="space-y-0.5">
                <p className="text-[12px] font-medium">Require Approval</p>
                <p className="text-xs text-muted-foreground">
                  Whether this service requires approval for programmatic
                  access.
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
                  htmlFor="approval-mode-select"
                >
                  Approval Mode
                </label>
                <Select
                  value={selectedApprovalMode}
                  onValueChange={(v) =>
                    setSelectedApprovalMode(v as "per_request" | "grant")
                  }
                >
                  <SelectTrigger id="approval-mode-select">
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
            <Button
              variant="outline"
              onClick={() => setAddServiceDialogOpen(false)}
            >
              Cancel
            </Button>
            <Button
              variant="primary"
              onClick={() => void handleAddServiceConfig()}
              disabled={!selectedServiceId}
              isLoading={setConfigMutation.isPending}
            >
              Add Override
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

