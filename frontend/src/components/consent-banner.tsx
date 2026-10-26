/** Explicit account permission plus a separately retained device restriction. */
import { useState } from 'react';
import { toast } from 'sonner';
import { useAuthStore } from '../stores/auth-store';
import { saveAnalyticsConsent } from '../lib/analytics-consent';
import { Button } from './ui/button';
import { useConsentStore } from '../stores/consent-store';
import { usePublicConfig } from '../hooks/use-public-config';

export function ConsentBanner() {
  const asked = useConsentStore((s) => s.asked);
  const accountId = useAuthStore((s) => s.user?.id);
  const [saving, setSaving] = useState(false);
  async function choose(enabled: boolean) {
    if (!accountId) return;
    setSaving(true);
    try { await saveAnalyticsConsent(accountId, enabled); }
    catch { toast.error('Could not save account analytics permission. This browser remains off.'); }
    finally { setSaving(false); }
  }
  // Only fetch config if the banner could possibly render. Once the
  // user has answered (asked=true), we never render regardless of
  // config, so the fetch would be wasted. TanStack Query dedupes,
  // so if another consumer (main.tsx) has already fetched, this is
  // a free cache read.
  const { data: cfg } = usePublicConfig({ enabled: !asked });

  if (asked || !accountId) return null;

  // Default-off contract: when the backend's /public/config reports
  // no telemetry DSN AND share-back is not opted in, the app has
  // nothing to capture — rendering the banner would be user-visible
  // drift from a pre-telemetry deploy. Short-circuit here. The banner
  // still renders normally on any deploy where telemetry could fire.
  const telemetryActive = !!(cfg?.telemetry_dsn || cfg?.telemetry_share_analytics);
  if (!telemetryActive) return null;

  return (
    <div
      role="dialog"
      aria-live="polite"
      aria-label="Telemetry consent"
      className="fixed inset-x-0 bottom-0 z-50 border-t bg-background/95 backdrop-blur"
    >
      <div className="mx-auto flex max-w-5xl flex-col gap-3 px-4 py-3 sm:flex-row sm:items-center sm:justify-between">
        <div className="text-12 text-muted-foreground">
          Optional usage analytics are linked to your account. Allow enables
          account permission and this browser. Declining withdraws account
          permission across devices. Each other device still needs its own
          opt-in, and Do Not Track always disables collection. Change this in
          Settings → Privacy.
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <Button
            variant="ghost"
            disabled={saving}
            onClick={() => void choose(false)}
            aria-label="Decline telemetry"
          >
            No Thanks
          </Button>
          <Button
            disabled={saving}
            onClick={() => void choose(true)}
            aria-label="Accept telemetry"
          >
            Allow
          </Button>
        </div>
      </div>
    </div>
  );
}
