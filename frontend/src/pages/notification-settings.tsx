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
import { useServices } from "@/hooks/use-services";
import {
  updateNotificationSettingsSchema,
  type UpdateNotificationSettingsFormData,
} from "@/schemas/approvals";
import { ApiError } from "@/lib/api-client";
import { PageHeader } from "@/components/shared/page-header";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
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
  Bell,
  MessageSquare,
  RotateCcw,
  Shield,
  Smartphone,
  Trash2,
  Unlink,
} from "lucide-react";
import { toast } from "sonner";

export function NotificationSettingsPage() {
  const { data: settings, isLoading, error } = useNotificationSettings();
  const updateMutation = useUpdateNotificationSettings();
  const telegramLinkMutation = useTelegramLink();
  const telegramDisconnectMutation = useTelegramDisconnect();

  const {
    data: pushDevices,
    isLoading: isPushDevicesLoading,
  } = usePushDevices();
  const removeDeviceMutation = useRemoveDevice();

  const { data: services } = useServices();
  const {
    data: serviceConfigs,
    isLoading: isServiceConfigsLoading,
    error: serviceConfigsError,
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

  // Services that already have per-service configs
  const configuredServiceIds = new Set(
    serviceConfigs?.configs.map((c) => c.service_id) ?? [],
  );

  // Available services for adding a new per-service config
  const availableServices =
    isServiceConfigsLoading || serviceConfigsError
      ? []
      : (services ?? []).filter(
          (s) => s.is_active && !configuredServiceIds.has(s.id),
        );

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
        err instanceof ApiError
          ? err.message
          : "Failed to disconnect Telegram",
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
      });
      toast.success("Per-service approval override added");
      setAddServiceDialogOpen(false);
      setSelectedServiceId("");
      setSelectedApprovalRequired(true);
    } catch (err) {
      toast.error(
        err instanceof ApiError
          ? err.message
          : "Failed to add service config",
      );
    }
  }

  async function handleToggleServiceConfig(
    serviceId: string,
    approvalRequired: boolean,
  ) {
    try {
      await setConfigMutation.mutateAsync({
        serviceId,
        approvalRequired,
      });
    } catch (err) {
      toast.error(
        err instanceof ApiError
          ? err.message
          : "Failed to update service config",
      );
    }
  }

  async function handleDeleteServiceConfig(serviceId: string) {
    try {
      await deleteConfigMutation.mutateAsync(serviceId);
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
        <div className="flex flex-col items-center justify-center py-12 text-center">
          <Bell className="mb-4 h-12 w-12 text-muted-foreground/50" />
          <p className="text-sm text-muted-foreground">
            Failed to load notification settings. Please try again.
          </p>
        </div>
      ) : (
        <div className="space-y-6">
          {/* Telegram Connection Card */}
          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2">
                <MessageSquare className="h-5 w-5" aria-hidden="true" />
                Telegram Connection
              </CardTitle>
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
                        <span className="text-sm text-muted-foreground">
                          {settings.telegram_username}
                        </span>
                      )}
                    </>
                  ) : (
                    <Badge variant="outline">Not connected</Badge>
                  )}
                </div>
                {settings?.telegram_connected ? (
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => setDisconnectDialogOpen(true)}
                  >
                    <Unlink className="mr-1 h-4 w-4" />
                    Disconnect
                  </Button>
                ) : (
                  <Button
                    size="sm"
                    onClick={() => void handleLinkTelegram()}
                    isLoading={telegramLinkMutation.isPending}
                  >
                    <MessageSquare className="mr-1 h-4 w-4" />
                    Connect Telegram
                  </Button>
                )}
              </div>
            </CardContent>
          </Card>

          {/* Push Devices Card */}
          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2">
                <Smartphone className="h-5 w-5" aria-hidden="true" />
                Push Devices
              </CardTitle>
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
                <p className="py-4 text-center text-sm text-muted-foreground">
                  No devices registered. Install the NyxID mobile app and sign
                  in to register a device.
                </p>
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
                            device.platform === "apns" ? "outline" : "secondary"
                          }
                        >
                          {device.platform === "apns" ? "iOS" : "Android"}
                        </Badge>
                        <div className="space-y-0.5">
                          <p className="text-sm font-medium">
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
                        <Trash2 className="h-4 w-4 text-muted-foreground" />
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
              <CardTitle className="flex items-center gap-2">
                <Bell className="h-5 w-5" aria-hidden="true" />
                Approval Preferences
              </CardTitle>
              <CardDescription>
                Configure whether approval is required and how long grants last.
              </CardDescription>
            </CardHeader>
            <CardContent>
              <Form {...form}>
                <form
                  onSubmit={form.handleSubmit((data) =>
                    void handleSave(data),
                  )}
                  className="space-y-6"
                >
                  {form.formState.errors.root && (
                    <div
                      role="alert"
                      className="rounded-md bg-destructive/10 p-3 text-sm text-destructive"
                    >
                      {form.formState.errors.root.message}
                    </div>
                  )}

                  <FormField
                    control={form.control}
                    name="approval_required"
                    render={({ field }) => (
                      <FormItem className="flex items-center justify-between rounded-lg border border-border p-4">
                        <div className="space-y-0.5">
                          <FormLabel className="text-base">
                            Require Approval (Global Default)
                          </FormLabel>
                          <FormDescription>
                            When enabled, proxy and LLM gateway requests using
                            your credentials require explicit approval.
                            Per-service overrides below can exempt or add
                            specific services.
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
                      <FormItem className="flex items-center justify-between rounded-lg border border-border p-4">
                        <div className="space-y-0.5">
                          <FormLabel className="text-base">
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
                      <FormItem className="flex items-center justify-between rounded-lg border border-border p-4">
                        <div className="space-y-0.5">
                          <FormLabel className="text-base">
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
                            disabled={
                              !pushDevices?.devices.length
                            }
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
                          re-prompting (1-365 days).
                        </FormDescription>
                        <FormMessage />
                      </FormItem>
                    )}
                  />

                  <Button
                    type="submit"
                    isLoading={updateMutation.isPending}
                  >
                    Save Preferences
                  </Button>
                </form>
              </Form>
            </CardContent>
          </Card>

          {/* Per-Service Approval Overrides */}
          <Card>
            <CardHeader>
              <div className="flex items-center justify-between">
                <div>
                  <CardTitle className="flex items-center gap-2">
                    <Shield className="h-5 w-5" aria-hidden="true" />
                    Per-Service Approval Overrides
                  </CardTitle>
                  <CardDescription>
                    Override the global approval setting for specific services.
                    Services without an override use the global default above.
                  </CardDescription>
                </div>
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => setAddServiceDialogOpen(true)}
                  disabled={
                    isServiceConfigsLoading ||
                    Boolean(serviceConfigsError) ||
                    availableServices.length === 0
                  }
                >
                  Add Override
                </Button>
              </div>
            </CardHeader>
            <CardContent>
              {isServiceConfigsLoading ? (
                <div className="space-y-3 py-2">
                  <Skeleton className="h-16 w-full" />
                  <Skeleton className="h-16 w-full" />
                </div>
              ) : serviceConfigsError ? (
                <p className="py-4 text-center text-sm text-muted-foreground">
                  Failed to load per-service overrides. Try refreshing the page.
                </p>
              ) : serviceConfigs?.configs.length === 0 ? (
                <p className="py-4 text-center text-sm text-muted-foreground">
                  No per-service overrides configured. All services use the
                  global default.
                </p>
              ) : (
                <div className="space-y-3">
                  {serviceConfigs?.configs.map((config) => (
                    <div
                      key={config.service_id}
                      className="flex items-center justify-between rounded-lg border border-border p-4"
                    >
                      <div className="space-y-0.5">
                        <p className="text-sm font-medium">
                          {config.service_name}
                        </p>
                        <p className="text-xs text-muted-foreground">
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
                              config.service_id,
                              checked,
                            )
                          }
                        />
                        <Button
                          variant="ghost"
                          size="icon"
                          onClick={() =>
                            void handleDeleteServiceConfig(config.service_id)
                          }
                          title="Remove override (use global default)"
                        >
                          <RotateCcw className="h-4 w-4 text-muted-foreground" />
                        </Button>
                      </div>
                    </div>
                  ))}
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
            <Button
              variant="outline"
              onClick={() => setRemoveDeviceId(null)}
            >
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
              <label className="text-sm font-medium" htmlFor="service-select">
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
                  {availableServices.map((s) => (
                    <SelectItem key={s.id} value={s.id}>
                      {s.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="flex items-center justify-between rounded-lg border border-border p-4">
              <div className="space-y-0.5">
                <p className="text-sm font-medium">Require Approval</p>
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
          </div>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setAddServiceDialogOpen(false)}
            >
              Cancel
            </Button>
            <Button
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
