import { getFormattingLocale } from "@/lib/formatting-locale";
import { useUiTranslation } from "@/i18n/use-ui-translation";
import { useEffect, useState } from "react";
import { Ban, ShieldX, Terminal } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ErrorBanner } from "@/components/shared/error-banner";
import { useLoginCodeAction, useLoginCodeStatus } from "@/hooks/use-login-code";
import type { LoginCode } from "@/schemas/login-code";

export function LoginCodeStatus({ issued, onClearCode, onNew }: { issued: LoginCode; onClearCode: () => void; onNew: () => void }) {
  const { t } = useUiTranslation();
  const status = useLoginCodeStatus(issued.request_id);
  const action = useLoginCodeAction(issued.request_id);
  const [now, setNow] = useState(() => Date.now());
  const expired = now >= new Date(issued.expires_at).getTime();
  const pending = !expired && (!status.data || status.data.status === "pending");
  useEffect(() => { const timer = setInterval(() => setNow(Date.now()), 1000); return () => clearInterval(timer); }, []);
  useEffect(() => { if (!pending) onClearCode(); }, [pending, onClearCode]);
  return <section className="space-y-4" aria-live="polite">
    <h2 className="text-15 font-semibold">{pending ? t("One-time terminal login") : status.data?.status === "redeemed" ? t("Login redeemed") : t("Login code closed")}</h2>
    {pending && <>
      <code data-sensitive className="block text-center font-mono text-28">{issued.code}</code>
      <p className="text-12 text-muted-foreground">{t("Expires ")}{new Date(issued.expires_at).toLocaleTimeString(getFormattingLocale())}</p>
      <p className="flex items-center gap-2 text-12"><Terminal className="size-4" /><code>nyxid login --code</code></p>
    </>}
    {status.data?.redeemed_at && <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 text-12">
      <dt>{t("Device")}</dt><dd className="break-all">{status.data.client_label ?? t("Not provided")}</dd>
      <dt>{t("IP address")}</dt><dd>{status.data.client_ip ?? t("Unavailable")} ({status.data.client_ip_attribution})</dd>
      <dt>{t("Redeemed")}</dt><dd>{new Date(status.data.redeemed_at).toLocaleString(getFormattingLocale())}</dd>
      <dt>{t("Access")}</dt><dd>{status.data.auth_kind === "agent_key" ? t("Restricted Agent Key") : t("Full account session")}</dd>
    </dl>}
    {(status.isError || action.isError) && <ErrorBanner message={t("Could not update login status. Try again.")} />}
    {pending && <Button variant="outline" isLoading={action.isPending} onClick={() => void action.mutateAsync("cancel").then(() => status.refetch()).catch(() => {})}>
      <Ban className="size-3" />{t("Cancel code")}</Button>}
    {status.data?.can_revoke && <Button variant="destructive" isLoading={action.isPending} onClick={() => void action.mutateAsync("revoke").then(() => status.refetch()).catch(() => {})}>
      <ShieldX className="size-3" />{t("Revoke login")}</Button>}
    {!pending && <Button variant="outline" onClick={onNew}>{t("Generate another code")}</Button>}
  </section>;
}
