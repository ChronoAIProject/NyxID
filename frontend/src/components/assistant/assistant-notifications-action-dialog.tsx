import { useEffect, useRef, useState } from "react";
import { Bell, MessageCircle, ShieldAlert } from "lucide-react";
import { z } from "zod";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { api } from "@/lib/api-client";
import {
  assertSecretFreeReadBack,
  assertNoSensitiveActionParams,
  errorMessage,
  isNewerTimestamp,
} from "./assistant-action-dialog-utils";

const notificationActions = ["update", "telegram_link", "telegram_disconnect"] as const;
export type AssistantNotificationsAction = (typeof notificationActions)[number];

const evidenceSchema = z
  .object({
    id: z.string().min(1),
    telegram_connected: z.boolean(),
    telegram_link_pending: z.boolean(),
    telegram_enabled: z.boolean(),
    approval_required: z.boolean(),
    approval_timeout_secs: z.number().int().min(10).max(300),
    grant_expiry_days: z.number().int().min(1).max(365),
    push_enabled: z.boolean(),
    push_device_count: z.number().int().nonnegative(),
    updated_at: z.string(),
  })
  .strict();
type NotificationEvidence = z.infer<typeof evidenceSchema>;

const responseSchema = z
  .object({
    resource: z.object({ bindingId: z.string().min(1) }).strict(),
    replayed: z.boolean(),
    linkCode: z.string().min(1).optional(),
    botUsername: z.string().min(1).optional(),
    expiresInSecs: z.number().int().positive().optional(),
  })
  .strict();

async function readEvidence(): Promise<NotificationEvidence> {
  const raw = await api.get<unknown>("/notifications/settings/authorization");
  assertSecretFreeReadBack(raw);
  return evidenceSchema.parse(raw);
}

export function AssistantNotificationsActionDialog({
  open,
  onOpenChange,
  actionRequestId,
  action,
  params,
  onComplete,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly actionRequestId: string;
  readonly action: AssistantNotificationsAction;
  readonly params: Record<string, unknown>;
  readonly onComplete: (bindingId: string) => void;
}) {
  const [before, setBefore] = useState<NotificationEvidence | null>(null);
  const [telegramEnabled, setTelegramEnabled] = useState(false);
  const [approvalRequired, setApprovalRequired] = useState(false);
  const [approvalTimeoutSecs, setApprovalTimeoutSecs] = useState("30");
  const [grantExpiryDays, setGrantExpiryDays] = useState("30");
  const [pushEnabled, setPushEnabled] = useState(false);
  const [confirmed, setConfirmed] = useState(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<{ id: string; linkCode?: string; botUsername?: string } | null>(null);
  const pendingRef = useRef(false);
  const destructive = action === "telegram_disconnect";

  useEffect(() => {
    if (!open) return;
    let active = true;
    void readEvidence()
      .then((evidence) => {
        if (!active) return;
        setBefore(evidence);
        setTelegramEnabled(evidence.telegram_enabled);
        setApprovalRequired(evidence.approval_required);
        setApprovalTimeoutSecs(String(evidence.approval_timeout_secs));
        setGrantExpiryDays(String(evidence.grant_expiry_days));
        setPushEnabled(evidence.push_enabled);
        setError(null);
      })
      .catch((caught: unknown) => {
        if (active) setError(errorMessage(caught, "NyxID could not load notification evidence."));
      });
    return () => { active = false; };
  }, [open]);

  function close() {
    pendingRef.current = false;
    setPending(false);
    setError(null);
    setConfirmed(false);
    setResult(null);
    setBefore(null);
    onOpenChange(false);
  }

  async function submit() {
    if (pendingRef.current || result) return;
    if (!before) {
      setError("Notification evidence must load before this action can run.");
      return;
    }
    if (destructive && !confirmed) {
      setError("Confirm this destructive change to continue.");
      return;
    }
    pendingRef.current = true;
    setPending(true);
    setError(null);
    try {
      assertNoSensitiveActionParams(params);
      const payload: Record<string, unknown> = { actionRequestId };
      if (action === "update") {
        payload.telegramEnabled = telegramEnabled;
        payload.approvalRequired = approvalRequired;
        payload.approvalTimeoutSecs = Number(approvalTimeoutSecs);
        payload.grantExpiryDays = Number(grantExpiryDays);
        payload.pushEnabled = pushEnabled;
      }
      const response = responseSchema.parse(
        await api.post<unknown>(
          `/assistant/actions/org/notifications/${action.replaceAll("_", "-")}`,
          payload,
        ),
      );
      const after = await readEvidence();
      if (response.resource.bindingId !== after.id) {
        throw new Error("NyxID returned a different notification binding.");
      }
      if (!response.replayed && !isNewerTimestamp(before.updated_at, after.updated_at)) {
        throw new Error("NyxID did not show a newer notification state.");
      }
      if (action === "update") {
        if (
          after.telegram_enabled !== telegramEnabled
          || after.approval_required !== approvalRequired
          || after.approval_timeout_secs !== Number(approvalTimeoutSecs)
          || after.grant_expiry_days !== Number(grantExpiryDays)
          || after.push_enabled !== pushEnabled
        ) {
          throw new Error("NyxID notification evidence does not match the requested settings.");
        }
      }
      if (action === "telegram_link") {
        if (!after.telegram_link_pending) {
          throw new Error("NyxID did not show a pending Telegram link.");
        }
        if (!response.replayed && (!response.linkCode || !response.botUsername)) {
          throw new Error("NyxID did not return the one-time Telegram link code to the browser.");
        }
      }
      if (action === "telegram_disconnect" && (after.telegram_connected || after.telegram_enabled || after.telegram_link_pending)) {
        throw new Error("NyxID still reports an active Telegram notification binding.");
      }
      setResult({
        id: after.id,
        ...(response.linkCode ? { linkCode: response.linkCode } : {}),
        ...(response.botUsername ? { botUsername: response.botUsername } : {}),
      });
      setBefore(after);
    } catch (caught) {
      setError(errorMessage(caught, "NyxID could not complete this notification action."));
    } finally {
      pendingRef.current = false;
      setPending(false);
    }
  }

  const title = result
    ? action === "telegram_link" ? "Telegram link ready" : "Notification change confirmed"
    : action === "update" ? "Update notifications" : action === "telegram_link" ? "Link Telegram" : "Disconnect Telegram";

  return (
    <Dialog open={open} onOpenChange={(next) => !next && close()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">{action === "update" ? <Bell className="size-4" /> : <MessageCircle className="size-4" />}{title}</DialogTitle>
          <DialogDescription>{result?.linkCode ? "Use this code now; it is available only from this browser response." : result ? "The canonical notification projection confirms the requested state." : destructive ? "Disconnecting Telegram is confirmed every time." : "NyxID keeps notification choices in your signed-in browser session."}</DialogDescription>
        </DialogHeader>
        {result ? (
          <div className="space-y-3 border-y border-border py-4">
            {result.linkCode ? <><div className="space-y-2"><Label htmlFor="telegram-link-code">One-time link code</Label><Input id="telegram-link-code" readOnly value={result.linkCode} className="font-mono" /></div><p className="text-xs text-muted-foreground">@{result.botUsername}</p></> : null}
            <p className="font-mono text-xs text-muted-foreground">{result.id}</p>
          </div>
        ) : (
          <div className="space-y-4 border-y border-border py-4">
            {action === "update" ? <><label className="flex items-center gap-2 text-sm"><Checkbox checked={telegramEnabled} onCheckedChange={(value) => setTelegramEnabled(value === true)} />Telegram notifications</label><label className="flex items-center gap-2 text-sm"><Checkbox checked={pushEnabled} onCheckedChange={(value) => setPushEnabled(value === true)} />Push notifications</label><label className="flex items-center gap-2 text-sm"><Checkbox checked={approvalRequired} onCheckedChange={(value) => setApprovalRequired(value === true)} />Require approval</label><div className="grid grid-cols-2 gap-3"><div className="space-y-2"><Label htmlFor="approval-timeout">Timeout (seconds)</Label><Input id="approval-timeout" type="number" min={10} max={300} value={approvalTimeoutSecs} onChange={(event) => setApprovalTimeoutSecs(event.target.value)} /></div><div className="space-y-2"><Label htmlFor="grant-expiry">Grant expiry (days)</Label><Input id="grant-expiry" type="number" min={1} max={365} value={grantExpiryDays} onChange={(event) => setGrantExpiryDays(event.target.value)} /></div></div></> : <p className="font-mono text-xs text-muted-foreground">{before?.id ?? "Loading"}</p>}
            {destructive ? <label className="flex items-start gap-2 text-xs"><Checkbox checked={confirmed} onCheckedChange={(value) => setConfirmed(value === true)} /><span className="flex items-center gap-1"><ShieldAlert className="size-3" />I understand Telegram will be disconnected.</span></label> : null}
          </div>
        )}
        {error ? <p role="alert" className="text-xs text-destructive">{error}</p> : null}
        <DialogFooter>{result ? <Button type="button" onClick={() => { onComplete(result.id); close(); }}>Done</Button> : <><Button type="button" variant="outline" onClick={close}>Cancel</Button><Button type="button" variant={destructive ? "destructive" : "primary"} isLoading={pending} disabled={pending || !before || (destructive && !confirmed)} onClick={() => void submit()}>{destructive ? "Disconnect" : "Continue"}</Button></>}</DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
