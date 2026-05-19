import { useState, useCallback } from "react";
import { Link } from "@tanstack/react-router";
import { Button, ButtonIcon } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Bell,
  Download,
  ExternalLink,
  MessageSquare,
  Shield,
  Smartphone,
  X,
} from "lucide-react";
import { toast } from "sonner";
import {
  useNotificationSettings,
  useUpdateNotificationSettings,
  useTelegramLink,
  usePushDevices,
} from "@/hooks/use-approvals";

const MOBILE_APP_LINK = "https://nyxid.onelink.me/REzJ/dql9w8fx";
const DISMISSED_KEY = "nyxid:notification-setup-dismissed";

export function NotificationSetupCard() {
  const { data: settings, isLoading } = useNotificationSettings();
  const { data: pushDevices } = usePushDevices();
  const updateMutation = useUpdateNotificationSettings();
  const telegramLinkMutation = useTelegramLink();
  const [linkDialogOpen, setLinkDialogOpen] = useState(false);
  const [dismissed, setDismissed] = useState(
    () => localStorage.getItem(DISMISSED_KEY) === "true",
  );

  const linkData = telegramLinkMutation.data;

  const telegramReady =
    settings?.telegram_connected && settings.telegram_enabled;
  const pushReady =
    settings?.push_enabled && (pushDevices?.devices.length ?? 0) > 0;
  const approvalEnabled = settings?.approval_required ?? false;
  const hasChannel = telegramReady || pushReady;
  const allDone = hasChannel && approvalEnabled;

  const handleDismiss = useCallback(() => {
    localStorage.setItem(DISMISSED_KEY, "true");
    setDismissed(true);
  }, []);

  async function handleLinkTelegram() {
    try {
      await telegramLinkMutation.mutateAsync();
      setLinkDialogOpen(true);
    } catch {
      toast.error("Failed to generate link code");
    }
  }

  async function handleToggleApproval(enable: boolean) {
    try {
      await updateMutation.mutateAsync({ approval_required: enable });
      toast.success(
        enable
          ? "Approval protection enabled"
          : "Approval protection disabled",
      );
    } catch {
      toast.error("Failed to update approval settings");
    }
  }

  // Once fully set up and dismissed, hide the card
  if (allDone && dismissed) return null;

  if (isLoading) {
    return <Skeleton className="h-48 w-full rounded-xl" />;
  }

  const steps = [
    {
      done: Boolean(telegramReady || pushReady),
      label: "Set up a notification channel",
    },
    { done: approvalEnabled, label: "Enable approval protection" },
  ];

  return (
    <>
      <div className="relative rounded-xl border border-border/50 bg-transparent p-4">
        {allDone && (
          <Button
            variant="ghost"
            className="absolute right-2 top-2 h-7 w-7 p-0 text-muted-foreground hover:text-foreground"
            onClick={handleDismiss}
            aria-label="Dismiss"
          >
            <X className="h-3.5 w-3.5" />
          </Button>
        )}

        <div className="flex flex-col gap-5">
          {/* Header */}
          <div className="flex items-center gap-3">
            <Bell className="h-5 w-5 text-primary" aria-hidden="true" />
            <div className="flex flex-col gap-0.5">
              <h3 className="text-[22px] font-normal leading-tight">
                Notifications & Approvals
              </h3>
              <p className="text-[12px] text-muted-foreground">
                Control how AI agents access your services
              </p>
            </div>
          </div>

          {/* Progress steps */}
          <div className="flex flex-col gap-2">
            {steps.map((step, i) => (
              <div key={i} className="flex items-center gap-2.5">
                <div className="flex h-5 w-5 shrink-0 items-center justify-center rounded-full border border-white/[0.08] bg-white/[0.04] text-[10px] font-medium text-muted-foreground">
                  {i + 1}
                </div>
                <span
                  className={
                    step.done
                      ? "text-[13px] text-muted-foreground"
                      : "text-[13px] font-medium"
                  }
                >
                  {step.label}
                </span>
              </div>
            ))}
          </div>

          {/* Success banner when approval is active */}
          {allDone && (
            <div className="rounded-lg border border-primary/20 bg-primary/5 px-4 py-3">
              <div className="flex items-center gap-2">
                <Shield className="h-4 w-4 text-primary" />
                <span className="text-[13px] font-medium">
                  Approval protection is active
                </span>
              </div>
              <p className="mt-1 text-xs text-muted-foreground">
                AI agents must request your approval before accessing services.{" "}
                <Link
                  to="/approvals/settings"
                  className="text-primary hover:underline"
                >
                  Manage settings
                </Link>
              </p>
            </div>
          )}

          {/* Channel rows -- always shown so users can add more channels */}
          <div className="flex flex-col gap-3">
            {/* Telegram */}
            <div className="flex items-center justify-between rounded-lg border border-border px-4 py-3">
              <div className="flex items-center gap-3">
                <MessageSquare
                  className="h-4 w-4 text-muted-foreground"
                  aria-hidden="true"
                />
                <div>
                  <p className="text-[13px] font-medium">Telegram</p>
                  <p className="text-xs text-muted-foreground">
                    Receive approval requests via bot
                  </p>
                </div>
              </div>
              {telegramReady ? (
                <Badge variant="success">Connected</Badge>
              ) : (
                <Button
                  variant="outline"
                  onClick={() => void handleLinkTelegram()}
                  isLoading={telegramLinkMutation.isPending}
                >
                  <ButtonIcon><MessageSquare className="h-3.5 w-3.5" /></ButtonIcon>
                  Connect
                </Button>
              )}
            </div>

            {/* Mobile App */}
            <div className="flex items-center justify-between rounded-lg border border-border px-4 py-3">
              <div className="flex items-center gap-3">
                <Smartphone
                  className="h-4 w-4 text-muted-foreground"
                  aria-hidden="true"
                />
                <div>
                  <p className="text-[13px] font-medium">NyxID Mobile App</p>
                  <p className="text-xs text-muted-foreground">
                    Approve from your phone (iOS & Android)
                  </p>
                </div>
              </div>
              {pushReady ? (
                <Badge variant="success">
                  {pushDevices?.devices.length ?? 0} device
                  {(pushDevices?.devices.length ?? 0) !== 1 ? "s" : ""}
                </Badge>
              ) : (
                <Button variant="outline" asChild>
                  <a
                    href={MOBILE_APP_LINK}
                    target="_blank"
                    rel="noopener noreferrer"
                  >
                    <ButtonIcon><Download className="h-3.5 w-3.5" /></ButtonIcon>
                    Download
                    <ExternalLink className="h-3 w-3" />
                  </a>
                </Button>
              )}
            </div>

            {/* Enable approval button */}
            {hasChannel && !approvalEnabled && (
              <Button
                variant="primary"
                className="w-full"
                onClick={() => void handleToggleApproval(true)}
                isLoading={updateMutation.isPending}
              >
                <ButtonIcon><Shield className="h-4 w-4" /></ButtonIcon>
                Enable Approval Protection
              </Button>
            )}

            {!hasChannel && (
              <p className="text-xs text-muted-foreground">
                Connect Telegram or download the mobile app to enable approval
                protection. Approval is enabled automatically when you set up a
                channel.
              </p>
            )}
          </div>
        </div>
      </div>

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
    </>
  );
}
