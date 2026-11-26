import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { translateErrorMessage } from "@/lib/error-messages";
import { getFormattingLocale } from "@/lib/formatting-locale";
import { useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  useNotificationSettings,
  useUpdateNotificationSettings,
  useTelegramLink,
  useTelegramDisconnect,
  usePushDevices,
  useRemoveDevice,
} from "@/hooks/use-approvals";
import {
  updateNotificationSettingsSchema,
  type UpdateNotificationSettingsFormData,
} from "@/schemas/approvals";
import { ApiError } from "@/lib/api-client";
import { ErrorBanner } from "@/components/shared/error-banner";
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
  useAppForm,
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
  Smartphone,
  Trash2,
  Unlink,
} from "lucide-react";
import { toast } from "sonner";
import { Link } from "@tanstack/react-router";
import { ApprovalSetupWizard } from "@/components/shared/approval-setup-wizard";

export function NotificationSettings() {
  useUiTranslation();
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

  const [linkDialogOpen, setLinkDialogOpen] = useState(false);
  const [disconnectDialogOpen, setDisconnectDialogOpen] = useState(false);
  const [removeDeviceId, setRemoveDeviceId] = useState<string | null>(null);
  const linkData = telegramLinkMutation.data;

  const form = useAppForm<UpdateNotificationSettingsFormData>({
    resolver: zodResolver(updateNotificationSettingsSchema),
    values: settings
      ? {
          telegram_enabled: settings.telegram_enabled,
          push_enabled: settings.push_enabled,
          group_mentions_enabled: settings.group_mentions_enabled ?? true,
          global_approval_mode: settings.global_approval_mode ?? (settings.approval_required ? "all" : "off"),
          approval_timeout_secs: settings.approval_timeout_secs,
          grant_expiry_days: settings.grant_expiry_days,
        }
      : undefined,
  });

  async function handleSave(data: UpdateNotificationSettingsFormData) {
    try {
      const update = { ...data };
      // An older replica may omit a saved preference. Do not overwrite it
      // while saving an unrelated setting during a rolling deployment.
      if (settings?.group_mentions_enabled === undefined && !form.getFieldState("group_mentions_enabled").isDirty) {
        delete update.group_mentions_enabled;
      }
      await updateMutation.mutateAsync(update);
      toast.success(translateUi("Notification settings updated"));
    } catch (err) {
      if (err instanceof ApiError) {
        form.setError("root", { message: err.message });
      } else {
        toast.error(translateErrorMessage("Failed to update settings"));
      }
    }
  }

  async function handleLinkTelegram() {
    try {
      await telegramLinkMutation.mutateAsync();
      setLinkDialogOpen(true);
    } catch (err) {
      toast.error(
        translateErrorMessage(err instanceof ApiError ? err.message : "Failed to generate link code"),
      );
    }
  }

  async function handleDisconnect() {
    try {
      await telegramDisconnectMutation.mutateAsync();
      toast.success(translateUi("Telegram disconnected"));
    } catch (err) {
      toast.error(
        translateErrorMessage(err instanceof ApiError ? err.message : "Failed to disconnect Telegram"),
      );
    } finally {
      setDisconnectDialogOpen(false);
    }
  }

  async function handleRemoveDevice() {
    if (!removeDeviceId) return;

    try {
      await removeDeviceMutation.mutateAsync(removeDeviceId);
      toast.success(translateUi("Device removed"));
    } catch (err) {
      toast.error(
        translateErrorMessage(err instanceof ApiError ? err.message : "Failed to remove device"),
      );
    } finally {
      setRemoveDeviceId(null);
    }
  }

  return (
    <div className="space-y-8">
      {isLoading ? (
        <div className="space-y-4">
          <Skeleton className="h-48 w-full" />
          <Skeleton className="h-64 w-full" />
        </div>
      ) : error ? (
        <ErrorBanner message={translateUi("Failed to load notification settings. Please try again.")} onRetry={refetch} />
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
            approvalMode={settings?.global_approval_mode}
            approvalSuspended={Boolean(settings?.approval_suspended)}
            deliveryConfigured={settings?.approval_delivery_configured}
            webAvailable={settings?.approval_web_available}
          />

          {/* Telegram Connection Card */}
          <Card>
            <CardHeader>
              <CardTitle>{translateUi("Telegram Connection")}</CardTitle>
              <CardDescription>{translateUi("Connect your Telegram account to receive approval notifications.")}</CardDescription>
            </CardHeader>
            <CardContent>
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-3">
                  {settings?.telegram_connected ? (
                    <>
                      <Badge variant="success">{translateUi("Connected")}</Badge>
                      {settings.telegram_username && (
                        <span className="text-14 text-muted-foreground">
                          {settings.telegram_username}
                        </span>
                      )}
                    </>
                  ) : (
                    <Badge variant="secondary">{translateUi("Not connected")}</Badge>
                  )}
                </div>
                {settings?.telegram_connected ? (
                  <Button
                    variant="outline"
                    className="text-text-tertiary hover:text-muted-foreground"
                    onClick={() => setDisconnectDialogOpen(true)}
                  >
                    <ButtonIcon><Unlink className="h-3 w-3" /></ButtonIcon>{translateUi("Disconnect")}</Button>
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
                    </ButtonIcon>{translateUi("Connect Telegram")}</Button>
                )}
              </div>
            </CardContent>
          </Card>

          {/* Push Devices Card */}
          <Card>
            <CardHeader>
              <CardTitle>{translateUi("Push Devices")}</CardTitle>
              <CardDescription>{translateUi("Mobile devices registered for push notifications. Devices are registered from the NyxID mobile app.")}</CardDescription>
            </CardHeader>
            <CardContent>
              {isPushDevicesLoading ? (
                <div className="space-y-3 py-2">
                  <Skeleton className="h-14 w-full" />
                  <Skeleton className="h-14 w-full" />
                </div>
              ) : !pushDevices?.devices.length ? (
                <div className="space-y-3">
                  <div className="rounded-lg border border-dashed border-border px-4 py-3 text-14 text-muted-foreground">{translateUi("No devices registered. Install the NyxID mobile app and sign in to register a device.")}</div>
                  <div className="flex justify-end">
                    <Button variant="primary" asChild>
                      <a href="https://nyxid.dev/app" target="_blank" rel="noopener noreferrer">
                        <ButtonIcon className="border-white/20 bg-white/10">
                          <Smartphone className="h-3 w-3" />
                        </ButtonIcon>{translateUi("Get App")}</a>
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
                          {device.platform === "apns" ? translateUi("iOS") : translateUi("Android")}
                        </Badge>
                        <div className="space-y-0.5">
                          <p className="type-row-title">
                            {device.device_name ?? translateUi("Unknown device")}
                          </p>
                          <p className="text-13 text-muted-foreground">{translateUi("Registered")}{" "}
                            {new Date(
                              device.registered_at,
                            ).toLocaleDateString(getFormattingLocale())}
                            {device.last_used_at && (
                              <>
                                {" "}{translateUi("· Last used")}{" "}
                                {new Date(
                                  device.last_used_at,
                                ).toLocaleDateString(getFormattingLocale())}
                              </>
                            )}
                          </p>
                        </div>
                      </div>
                      <Button
                        variant="ghost"
                        size="icon"
                        onClick={() => setRemoveDeviceId(device.device_id)}
                        title={translateUi("Remove device")}
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
              <CardTitle>{translateUi("Approval Preferences")}</CardTitle>
              <CardDescription>{translateUi("Configure approval settings. When enabled, requests require approval by default (per-request mode). Auto-connected services remain available unless you add an explicit per-service policy. You can opt specific services into time-based grants below.")}</CardDescription>
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
                      className="rounded-lg bg-destructive/10 p-3 text-14 text-destructive"
                    >
                      {form.formState.errors.root.message}
                    </div>
                  )}

                  <FormField
                    control={form.control}
                    name="global_approval_mode"
                    render={({ field }) => (
                      <FormItem className="!space-y-0 flex items-center justify-between rounded-lg border border-border p-4">
                        <div className="space-y-0.5">
                          <FormLabel>{translateUi("Approval (Global Default)")}</FormLabel>
                          <FormDescription>{translateUi("Writes only requires approval for changes and destructive operations. Reads and stateless model inference can proceed. Per-service and organization policies still apply.")}</FormDescription>
                        </div>
                        <Select value={field.value ?? ""} onValueChange={field.onChange}>
                          <FormControl>
                            <SelectTrigger className="w-48" aria-label={translateUi("Approval (Global Default)")}>
                              <SelectValue />
                            </SelectTrigger>
                          </FormControl>
                          <SelectContent>
                            <SelectItem value="writes_only">{translateUi("Writes only (default)")}</SelectItem>
                            <SelectItem value="all">{translateUi("All requests")}</SelectItem>
                            <SelectItem value="off">{translateUi("Off")}</SelectItem>
                          </SelectContent>
                        </Select>
                      </FormItem>
                    )}
                  />

                  <FormField
                    control={form.control}
                    name="telegram_enabled"
                    render={({ field }) => (
                      <FormItem className="!space-y-0 flex items-center justify-between rounded-lg border border-border p-4">
                        <div className="space-y-0.5">
                          <FormLabel>{translateUi("Telegram Notifications")}</FormLabel>
                          <FormDescription>{translateUi("Send approval requests via Telegram.")}</FormDescription>
                        </div>
                        <FormControl>
                          <Switch
                            checked={field.value ?? false}
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
                          <FormLabel>{translateUi("Push Notifications")}</FormLabel>
                          <FormDescription>{translateUi("Send notifications to your registered devices.")}</FormDescription>
                        </div>
                        <FormControl>
                          <Switch
                            checked={field.value ?? false}
                            onCheckedChange={field.onChange}
                            disabled={!pushDevices?.devices.length}
                          />
                        </FormControl>
                      </FormItem>
                    )}
                  />

                  <FormField
                    control={form.control}
                    name="group_mentions_enabled"
                    render={({ field }) => (
                      <FormItem className="!space-y-0 flex items-center justify-between rounded-lg border border-border p-4">
                        <div className="space-y-0.5">
                          <FormLabel>{translateUi("Mentions in groups")}</FormLabel>
                          <FormDescription>{translateUi("Push alerts when someone mentions you in a group. Requires Push Notifications; unread badges stay on.")}</FormDescription>
                        </div>
                        <FormControl><Switch checked={field.value ?? true} onCheckedChange={field.onChange} /></FormControl>
                      </FormItem>
                    )}
                  />

                  <FormField
                    control={form.control}
                    name="approval_timeout_secs"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>{translateUi("Approval Timeout (seconds)")}</FormLabel>
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
                        <FormDescription>{translateUi("How long to wait for a response before auto-rejecting (10-300 seconds).")}</FormDescription>
                        <FormMessage />
                      </FormItem>
                    )}
                  />

                  <FormField
                    control={form.control}
                    name="grant_expiry_days"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>{translateUi("Grant Expiry (days)")}</FormLabel>
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
                        <FormDescription>{translateUi("How many days an approval grant lasts before re-prompting (1-365 days). Only applies to services configured with time-based grant mode.")}</FormDescription>
                        <FormMessage />
                      </FormItem>
                    )}
                  />

                  <div className="flex justify-end">
                    <Button variant="primary" type="submit" isLoading={updateMutation.isPending} disabled={!form.formState.isDirty || updateMutation.isPending}>{translateUi("Save Preferences")}</Button>
                  </div>
                </form>
              </Form>
            </CardContent>
          </Card>

          {/* Per-service approval settings moved to the assistant's Approvals page. */}
          <Card>
            <CardHeader>
              <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
                <div className="space-y-1.5">
                  <CardTitle>{translateUi("Approval rules for each service")}</CardTitle>
                  <CardDescription>{translateUi("Choosing which services and actions need your approval now lives on the Approvals page, under Rules. Rules you set here before still apply and appear there.")}</CardDescription>
                </div>
                <div className="shrink-0 self-start sm:self-center">
                  <Button asChild variant="outline">
                    <Link to="/assistant/approvals">{translateUi("Open approval rules")}</Link>
                  </Button>
                </div>
              </div>
            </CardHeader>
          </Card>
        </div>
      )}

      {/* Telegram Link Dialog */}
      <Dialog open={linkDialogOpen} onOpenChange={setLinkDialogOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{translateUi("Connect Telegram")}</DialogTitle>
            <DialogDescription>{translateUi("Send the following command to the NyxID bot on Telegram to link your account.")}</DialogDescription>
          </DialogHeader>
          {linkData && (
            <div className="space-y-4">
              <div className="rounded-lg bg-muted p-4 text-center">
                <p className="text-13 text-muted-foreground">{translateUi("Send this to @")}{linkData.bot_username}
                </p>
                <code className="mt-2 block type-body">
                  /start {linkData.link_code}
                </code>
              </div>
              <p className="text-13 text-muted-foreground">{translateUi("This code expires in")}{" "}
                {String(Math.floor(linkData.expires_in_secs / 60))}{translateUi(" minutes.")}</p>
            </div>
          )}
          <DialogFooter>
            <Button variant="outline" onClick={() => setLinkDialogOpen(false)}>{translateUi("Close")}</Button>
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
            <DialogTitle>{translateUi("Disconnect Telegram")}</DialogTitle>
            <DialogDescription>{translateUi("Are you sure you want to disconnect your Telegram account? You will no longer receive approval notifications via Telegram.")}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setDisconnectDialogOpen(false)}
            >{translateUi("Cancel")}</Button>
            <Button
              variant="destructive"
              onClick={() => void handleDisconnect()}
              isLoading={telegramDisconnectMutation.isPending}
            >{translateUi("Disconnect")}</Button>
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
            <DialogTitle>{translateUi("Remove Device")}</DialogTitle>
            <DialogDescription>{translateUi("Are you sure you want to remove this device? It will no longer receive push notifications. You can re-register it from the mobile app.")}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setRemoveDeviceId(null)}>{translateUi("Cancel")}</Button>
            <Button
              variant="destructive"
              onClick={() => void handleRemoveDevice()}
              isLoading={removeDeviceMutation.isPending}
            >{translateUi("Remove Device")}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
