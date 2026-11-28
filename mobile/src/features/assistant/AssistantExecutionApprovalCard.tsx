import { Linking } from "react-native";
import { useEffect, useRef, useState } from "react";
import { ActionButton, Card, Label } from "../../components/workspace/WorkspaceUI";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp } from "../../i18n/appTranslation";
import { useLocalizedState } from "../../i18n/useLocalizedState";
import { getFrontendUrl } from "../../lib/env";
import { assistantApprovalReviewPath } from "./assistantApprovalReview";
import type { AssistantHistory } from "./assistantTypes";

/** Execution proposals are reviewed on the web, independently of ordinary approvals. */
export function AssistantExecutionApprovalCard({ approval, disabled }: {
  approval: AssistantHistory["approvals"][number];
  disabled: boolean;
}) {
  useAppLanguage();
  const [error, setError] = useLocalizedState<string>();
  const [busy, setBusy] = useState(false);
  const opening = useRef(false);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => { alive.current = false; };
  }, []);
  const proposal = approval.execution_proposal;
  const path = assistantApprovalReviewPath(approval);
  const label = proposal?.state === "denied" ? "Request denied"
    : proposal?.state === "expired" ? "expired" : "Review in the web app";
  async function openReview() {
    const current = assistantApprovalReviewPath(approval);
    if (disabled || opening.current || !current) return;
    opening.current = true;
    setBusy(true);
    setError(undefined);
    try {
      await Linking.openURL(`${getFrontendUrl()}${current}`);
    } catch {
      if (alive.current) setError(() => translateApp("Could not open review. Try again."));
    } finally {
      opening.current = false;
      if (alive.current) setBusy(false);
    }
  }
  return <Card>
    <Label style={{ fontWeight: "600", fontSize: 13 }}>{translateApp(label)}</Label>
    <Label>{approval.service_name}: {approval.summary}</Label>
    {path ? <ActionButton
      label={translateApp("Review in the web app")}
      compact variant="secondary" disabled={disabled || busy} loading={busy}
      onPress={() => void openReview()}
    /> : <Label muted>{translateApp("This request is no longer available. Refresh its latest state.")}</Label>}
    {error ? <Label accessibilityRole="alert">{error}</Label> : null}
  </Card>;
}
