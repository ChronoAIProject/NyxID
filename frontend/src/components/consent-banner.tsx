import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { translateErrorMessage } from "@/lib/error-messages";
/** Explicit account permission plus a separately retained device restriction. */
import { useState } from 'react';
import { toast } from 'sonner';
import { useAuthStore } from '../stores/auth-store';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { readAnalyticsConsent, saveAnalyticsConsent } from '../lib/analytics-consent';
import { Button } from './ui/button';
import { useConsentStore } from '../stores/consent-store';
import { usePublicConfig } from '../hooks/use-public-config';

export function ConsentBanner() {
  useUiTranslation();
  const asked = useConsentStore((s) => s.asked);
  const devicePolicy = useConsentStore((s) => s.policy);
  const deviceEnabled = useConsentStore((s) => s.enabled);
  const [dismissed, setDismissed] = useState<string | null>(null);
  const accountId = useAuthStore((s) => s.user?.id);
  const [saving, setSaving] = useState(false);
  const queryClient = useQueryClient();
  const candidate = !!accountId && (!asked || deviceEnabled);
  const { data: cfg } = usePublicConfig({ enabled: candidate });
  const telemetryActive = !!(cfg?.telemetry_dsn || cfg?.telemetry_share_analytics);
  const permission = useQuery({
    queryKey: ["analytics-consent", accountId],
    queryFn: () => readAnalyticsConsent(accountId!),
    enabled: candidate && telemetryActive,
    staleTime: Infinity,
    gcTime: Infinity,
    retry: false,
    retryOnMount: false,
    refetchOnMount: false,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
  });
  const offered = permission.data?.diagnostics_offered === true;
  const current = offered && permission.data?.current_policy_version === "2" ? 2 : 1;
  const update = asked && deviceEnabled && permission.data?.enabled && current === 2 && (permission.data.policy_version !== "2" || devicePolicy !== 2);
  const dismissalKey = `nyxid.diagnostics.dismissed.${accountId}`;
  let previouslyDismissed = false;
  try { previouslyDismissed = localStorage.getItem(dismissalKey) === "2"; } catch { /* Session dismissal still works. */ }
  function dismiss() {
    try { localStorage.setItem(dismissalKey, "2"); } catch { /* No authority is granted by dismissal. */ }
    setDismissed(accountId ?? null);
  }
  async function choose(enabled: boolean) {
    if (!accountId) return;
    setSaving(true);
    try {
      const saved = await saveAnalyticsConsent(accountId, enabled, current);
      queryClient.setQueryData(["analytics-consent", accountId], saved);
    }
    catch { toast.error(translateErrorMessage('Could not save account analytics permission. This browser remains off.')); }
    finally { setSaving(false); }
  }
  if (!accountId || (asked && (!update || previouslyDismissed || dismissed === accountId))) return null;

  // Default-off contract: when the backend's /public/config reports
  // no telemetry DSN AND share-back is not opted in, the app has
  // nothing to capture — rendering the banner would be user-visible
  // drift from a pre-telemetry deploy. Short-circuit here. The banner
  // still renders normally on any deploy where telemetry could fire.
  if (!telemetryActive && current !== 2) return null;

  return (
    <div
      role="dialog"
      aria-live="polite"
      aria-label={translateUi("Telemetry consent")}
      className="fixed inset-x-0 bottom-0 z-50 border-t bg-background/95 backdrop-blur"
    >
      <div className="mx-auto flex max-w-5xl flex-col gap-3 px-4 py-3 sm:flex-row sm:items-center sm:justify-between">
        <div className="text-14 unthemed:text-12 text-muted-foreground">{current === 2 ? translateUi(update ? "Analytics updated: optional crash diagnostics include app version and sanitized code locations, without messages or URLs. Reports are kept for up to 30 days. Allow updated analytics for this account and browser, or keep your current choice." : "Optional usage analytics are linked to your account. Allow enables account permission and this browser. Declining withdraws account permission across devices. Each other device still needs its own opt-in, and Do Not Track or Global Privacy Control always disables collection. Optional crash diagnostics include app version and sanitized code locations, without messages or URLs; reports are kept for up to 30 days. Change this in Settings → Privacy.") : translateUi("Optional usage analytics are linked to your account. Allow enables account permission and this browser. Declining withdraws account permission across devices. Each other device still needs its own opt-in, and Do Not Track or Global Privacy Control always disables collection. Change this in Settings → Privacy.")}</div>
        <div className="flex shrink-0 items-center gap-2">
          <Button
            variant="ghost"
            disabled={saving}
            onClick={() => update ? dismiss() : void choose(false)}
            aria-label={translateUi("Decline telemetry")}
          >{translateUi(update ? "Not now" : "No Thanks")}</Button>
          <Button
            disabled={saving}
            onClick={() => void choose(true)}
            aria-label={translateUi("Accept telemetry")}
          >{translateUi(update ? "Allow updated analytics" : "Allow")}</Button>
        </div>
      </div>
    </div>
  );
}
