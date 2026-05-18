/**
 * Telemetry consent banner.
 *
 * Rendered by the Root component when the user has never answered the
 * consent prompt (`useConsentStore.asked === false`). Offers a binary
 * opt-in / opt-out; either choice dismisses the banner for this browser.
 * Consent is scoped to the current browser — other devices, the mobile
 * app, and the CLI manage their own telemetry state. Users can reverse
 * their choice at any time from Settings → Privacy.
 *
 * See `docs/TELEMETRY.md` §7 + §D for privacy gate + consent model.
 */

import { Button } from './ui/button';
import { useConsentStore } from '../stores/consent-store';
import { usePublicConfig } from '../hooks/use-public-config';

export function ConsentBanner() {
  const asked = useConsentStore((s) => s.asked);
  const setConsent = useConsentStore((s) => s.setConsent);
  // Only fetch config if the banner could possibly render. Once the
  // user has answered (asked=true), we never render regardless of
  // config, so the fetch would be wasted. TanStack Query dedupes,
  // so if another consumer (main.tsx) has already fetched, this is
  // a free cache read.
  const { data: cfg } = usePublicConfig({ enabled: !asked });

  if (asked) return null;

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
        <div className="text-[12px] text-muted-foreground">
          We collect anonymous usage telemetry to help us improve NyxID.
          We never capture credentials, form content, or the contents of
          your requests. This choice applies to this browser only — other
          devices and the CLI manage their own telemetry settings. You
          can change this later in Settings.
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <Button
            variant="ghost"
            onClick={() => setConsent(false)}
            aria-label="Decline telemetry"
          >
            No Thanks
          </Button>
          <Button
            onClick={() => setConsent(true)}
            aria-label="Accept telemetry"
          >
            Allow
          </Button>
        </div>
      </div>
    </div>
  );
}
