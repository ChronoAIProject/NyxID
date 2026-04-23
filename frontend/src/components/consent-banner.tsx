/**
 * Telemetry consent banner.
 *
 * Rendered by the Root component when the user has never answered the
 * consent prompt (`useConsentStore.asked === false`). Offers a binary
 * opt-in / opt-out; either choice dismisses the banner permanently for
 * this browser. Users can reverse their choice later from Settings.
 *
 * See `docs/TELEMETRY_M1.md` §7 + §D for privacy gate + consent model.
 */

import { Button } from './ui/button';
import { useConsentStore } from '../stores/consent-store';

export function ConsentBanner() {
  const asked = useConsentStore((s) => s.asked);
  const setConsent = useConsentStore((s) => s.setConsent);

  // Default-off contract: when no telemetry DSN is configured AND
  // share-back is not opted in at build time, the app has nothing to
  // capture — rendering the consent banner would be user-visible
  // drift from a pre-telemetry build. Short-circuit here. The banner
  // still renders normally on any deploy where telemetry could
  // actually fire.
  const telemetryActive =
    !!import.meta.env.VITE_TELEMETRY_DSN ||
    import.meta.env.VITE_NYXID_SHARE_ANALYTICS === "true";
  if (!telemetryActive) return null;

  if (asked) return null;

  return (
    <div
      role="dialog"
      aria-live="polite"
      aria-label="Telemetry consent"
      className="fixed inset-x-0 bottom-0 z-50 border-t bg-background/95 backdrop-blur"
    >
      <div className="mx-auto flex max-w-5xl flex-col gap-3 px-4 py-3 sm:flex-row sm:items-center sm:justify-between">
        <div className="text-sm text-muted-foreground">
          We collect anonymous usage telemetry to help us improve NyxID.
          We never capture credentials, form content, or the contents of
          your requests. You can change this later in Settings.
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <Button
            variant="ghost"
            size="sm"
            onClick={() => setConsent(false)}
            aria-label="Decline telemetry"
          >
            No thanks
          </Button>
          <Button
            size="sm"
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
