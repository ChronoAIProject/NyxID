import { useAuthStore } from "@/stores/auth-store";
import { useEffect, useState } from "react";
import { MachineGrantEditor } from "./machine-grant-editor";
import { ErrorBanner } from "./error-banner";
import { machineGrantUnavailable, prepareMachineConsent } from "@/hooks/use-machine-grants";
import { readMachineConsentScope } from "@/schemas/machine-grants";

/** Selections are saved by the human endpoint and replaced by its signed token. */
export function MachineGrantConsent({ consentRequest, onPrepared, disabled = false, allowedServiceIds }: {
  readonly consentRequest: string;
  readonly onPrepared: (token: string | null) => void;
  readonly disabled?: boolean;
  readonly allowedServiceIds?: readonly string[];
}) {
  const ownerId = useAuthStore((state) => state.user?.id);
  const [reviewedOwner] = useState(ownerId);
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => { const timer = window.setInterval(() => setNow(Date.now()), 1000); return () => window.clearInterval(timer); }, []);
  useEffect(() => { if (ownerId !== reviewedOwner) onPrepared(null); }, [ownerId, reviewedOwner, onPrepared]);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string>();
  const [prepared, setPrepared] = useState(false);
  const [approvedExpiry, setApprovedExpiry] = useState<number>();
  const signed = readMachineConsentScope(consentRequest);
  const requestValid = Boolean(signed?.scope.split(/\s+/).includes("machine") && signed.expiresAt > now && ownerId === reviewedOwner);
  useEffect(() => {
    if (!approvedExpiry) return;
    const timer = window.setTimeout(() => { onPrepared(null); setPrepared(false); setError("The protected Machine selection expired. Review and save it again before approval."); }, Math.max(0, approvedExpiry - Date.now()));
    return () => window.clearTimeout(timer);
  }, [approvedExpiry, onPrepared]);
  return <section aria-labelledby="machine-consent-heading" className="space-y-4 border-t border-border py-5">
    <h2 id="machine-consent-heading" className="text-15 font-semibold">Machine access</h2>
    <p className="break-all text-11 text-muted-foreground">Protected OAuth client: {signed?.clientId ?? "Unavailable"}</p>
    <p className="text-12 text-muted-foreground">Choose the Machines this OAuth client may use. Save the selection before allowing access; the server binds it to this one-time consent request. Machine access does not allow purchases, top-ups, pause, resume or deletion.</p>
    {!requestValid ? <ErrorBanner message="The protected Machine consent request is missing or expired. Restart authorization from the application." /> : <MachineGrantEditor allowedServiceIds={allowedServiceIds} disabled={disabled} submitting={pending} submitLabel="Save Machine selection" onSelectionChange={() => { onPrepared(null); setPrepared(false); setApprovedExpiry(undefined); }} onSubmit={async (selection) => {
      setError(undefined); onPrepared(null); setPrepared(false); setPending(true);
      try {
        const token = await prepareMachineConsent(consentRequest, selection);
        const returned = readMachineConsentScope(token);
        if (!returned || returned.clientId !== signed?.clientId || !returned.scope.split(/\s+/).includes("machine") || returned.expiresAt <= Date.now()) throw new Error("The protected Machine selection could not be verified for this request. Restart authorization.");
        onPrepared(token); setPrepared(true); setApprovedExpiry(returned.expiresAt);
      } catch (err) { setError(machineGrantUnavailable(err) ? "Machine consent is not available on this server yet. Approval is blocked; return to the application or try again after server integration." : err instanceof Error ? err.message : "Could not save the protected Machine selection."); }
      finally { setPending(false); }
    }} />}
    {prepared && <p role="status" className="text-12 text-success">Machine selection saved. Review the remaining permissions and allow access below.</p>}
    {error && <ErrorBanner message={error} />}
  </section>;
}
