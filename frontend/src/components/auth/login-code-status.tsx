import { useEffect, useState } from "react";
import { Ban, ShieldX, Terminal } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ErrorBanner } from "@/components/shared/error-banner";
import { useLoginCodeAction, useLoginCodeStatus } from "@/hooks/use-login-code";
import type { LoginCode } from "@/schemas/login-code";

export function LoginCodeStatus({ issued, onClearCode, onNew }: { issued: LoginCode; onClearCode: () => void; onNew: () => void }) {
  const status = useLoginCodeStatus(issued.request_id);
  const action = useLoginCodeAction(issued.request_id);
  const [now, setNow] = useState(() => Date.now());
  const expired = now >= new Date(issued.expires_at).getTime();
  const pending = !expired && (!status.data || status.data.status === "pending");
  useEffect(() => { const timer = setInterval(() => setNow(Date.now()), 1000); return () => clearInterval(timer); }, []);
  useEffect(() => { if (!pending) onClearCode(); }, [pending, onClearCode]);
  return <section className="space-y-4" aria-live="polite">
    <h2 className="text-[15px] font-semibold">{pending ? "One-time terminal login" : status.data?.status === "redeemed" ? "Login redeemed" : "Login code closed"}</h2>
    {pending && <>
      <code data-sensitive className="block text-center font-mono text-[28px]">{issued.code}</code>
      <p className="text-[12px] text-muted-foreground">Expires {new Date(issued.expires_at).toLocaleTimeString()}</p>
      <p className="flex items-center gap-2 text-[12px]"><Terminal className="size-4" /><code>nyxid login --code</code></p>
    </>}
    {status.data?.redeemed_at && <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 text-[12px]">
      <dt>Device</dt><dd className="break-all">{status.data.client_label ?? "Not provided"}</dd>
      <dt>IP address</dt><dd>{status.data.client_ip ?? "Unavailable"} ({status.data.client_ip_attribution})</dd>
      <dt>Redeemed</dt><dd>{new Date(status.data.redeemed_at).toLocaleString()}</dd>
      <dt>Access</dt><dd>{status.data.auth_kind === "agent_key" ? "Restricted Agent Key" : "Full account session"}</dd>
    </dl>}
    {(status.isError || action.isError) && <ErrorBanner message="Could not update login status. Try again." />}
    {pending && <Button variant="outline" isLoading={action.isPending} onClick={() => void action.mutateAsync("cancel").then(() => status.refetch()).catch(() => {})}>
      <Ban className="size-3" />Cancel code</Button>}
    {status.data?.can_revoke && <Button variant="destructive" isLoading={action.isPending} onClick={() => void action.mutateAsync("revoke").then(() => status.refetch()).catch(() => {})}>
      <ShieldX className="size-3" />Revoke login</Button>}
    {!pending && <Button variant="outline" onClick={onNew}>Generate another code</Button>}
  </section>;
}
