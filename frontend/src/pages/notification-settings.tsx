import { useState } from "react";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  useNotificationSettings,
  useUpdateNotificationSettings,
  useTelegramLink,
  useTelegramDisconnect,
} from "@/hooks/use-approvals";
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
import { Bell, MessageSquare, Unlink } from "lucide-react";
import { toast } from "sonner";

export function NotificationSettingsPage() {
  const { data: settings, isLoading, error } = useNotificationSettings();
  const updateMutation = useUpdateNotificationSettings();
  const telegramLinkMutation = useTelegramLink();
  const telegramDisconnectMutation = useTelegramDisconnect();

  const [linkDialogOpen, setLinkDialogOpen] = useState(false);
  const [disconnectDialogOpen, setDisconnectDialogOpen] = useState(false);

  const linkData = telegramLinkMutation.data;

  const form = useForm<UpdateNotificationSettingsFormData>({
    resolver: zodResolver(updateNotificationSettingsSchema),
    values: settings
      ? {
          telegram_enabled: settings.telegram_enabled,
          approval_required: settings.approval_required,
          approval_timeout_secs: settings.approval_timeout_secs,
          grant_expiry_days: settings.grant_expiry_days,
        }
      : undefined,
  });

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
                            Require Approval
                          </FormLabel>
                          <FormDescription>
                            When enabled, proxy and LLM gateway requests using
                            your credentials require explicit approval.
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
                This code expires in {String(Math.floor(linkData.expires_in_secs / 60))} minutes.
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
    </div>
  );
}
