import { useState, useContext } from "react";
import { QueryClientContext } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";
import { Button } from "@/components/ui/button";
import type { KeyInfo } from "@/types/keys";
import { credentialVerificationMeta } from "@/lib/credential-verification";

export function CredentialCheck({ connection, canManage }: { connection: KeyInfo; canManage: boolean }) {
  useUiTranslation();
  const queryClient = useContext(QueryClientContext);
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const meta = credentialVerificationMeta(connection);
  if (!meta) return null;
  async function check() {
    setBusy(true);
    setFailed(false);
    try {
      await api.post(`/keys/${encodeURIComponent(connection.id)}/verify`, {});
      await queryClient?.invalidateQueries({ queryKey: ["keys"] });
    } catch { setFailed(true); }
    finally { setBusy(false); }
  }
  return <div className="space-y-2 text-13 text-text-secondary">
    <p>{translateUi(meta.tooltip)}</p>
    {connection.credential_verification?.reason_code && <p>{translateUi("Check result")}: <code>{connection.credential_verification.reason_code}</code></p>}
    {canManage && connection.credential_verification?.can_check && <Button variant="outline" disabled={busy} onClick={() => void check()}>{translateUi(busy ? "Checking credential…" : "Check credential")}</Button>}
    {failed && <p role="alert">{translateUi("Could not check the credential. Try again in a moment.")}</p>}
  </div>;
}
