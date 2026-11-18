import { useRef, useState } from "react";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { useUiTranslation, translateUi } from "@/i18n/use-ui-translation";
import { translateErrorMessage } from "@/lib/error-messages";
import { useAuthStore } from "@/stores/auth-store";
import { readAnalyticsConsent, saveAnalyticsConsent } from "@/lib/analytics-consent";
import { Button } from "@/components/ui/button";
import { useConsentStore } from "@/stores/consent-store";
import { usePublicConfig } from "@/hooks/use-public-config";

/** The account remembers this prompt; every device retains its own opt-in. */
export function ConsentBanner() {
  useUiTranslation();
  const asked = useConsentStore((s) => s.asked);
  const deviceAccountId = useConsentStore((s) => s.accountId);
  const user = useAuthStore((s) => s.user);
  const accountId = user?.id;
  const footerRef = useRef<HTMLDivElement>(null);
  const [savingAccountId, setSavingAccountId] = useState<string | null>(null);
  const saving = !!accountId && savingAccountId === accountId;
  // Read acknowledgement with the authenticated profile so a saved decision
  // never flashes a prompt while a separate preference request is loading.
  // A prior express choice on this browser also counts when an older profile
  // has no account acknowledgement. Neither signal grants a device opt-in.
  const answered = user?.profile_config?.analytics?.prompt_seen === true
    || (deviceAccountId === accountId && asked);
  const { data: cfg } = usePublicConfig({ enabled: !!accountId && !answered });
  const telemetryActive = !!(cfg?.telemetry_dsn || cfg?.telemetry_share_analytics);
  const queryClient = useQueryClient();
  const permission = useQuery({
    queryKey: ["analytics-consent", accountId],
    queryFn: () => readAnalyticsConsent(accountId!),
    enabled: !!accountId && !answered && cfg?.crash_diagnostics_offered === true,
    staleTime: Infinity,
    gcTime: Infinity,
    retry: false,
    retryOnMount: false,
    refetchOnMount: false,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
  });
  const offered = cfg?.crash_diagnostics_offered === true && permission.data?.diagnostics_offered === true;
  const policy = offered && permission.data?.current_policy_version === "2" ? 2 : 1;
  const discovering = cfg?.crash_diagnostics_offered === true && permission.isPending;

  async function choose(enabled: boolean) {
    if (!accountId || saving) return;
    // Keep focus inside the footer before both choice buttons become disabled.
    footerRef.current?.focus();
    setSavingAccountId(accountId);
    try {
      const saved = await saveAnalyticsConsent(accountId, enabled, policy);
      queryClient.setQueryData(["analytics-consent", accountId], saved);
    } catch {
      if (useAuthStore.getState().user?.id === accountId) {
        toast.error(translateErrorMessage("Could not save account analytics permission. This browser remains off."));
      }
    } finally {
      setSavingAccountId((current) => current === accountId ? null : current);
    }
  }

  if (((answered || permission.data?.prompt_seen === true) && !saving)
      || !accountId || discovering || (!telemetryActive && policy !== 2)) return null;

  return (
    <DialogPrimitive.Root open>
      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay className="fixed inset-0 z-[199] bg-black/50" />
        <DialogPrimitive.Content
          ref={footerRef}
          aria-modal="true"
          aria-busy={saving}
          onEscapeKeyDown={(event) => event.preventDefault()}
          onPointerDownOutside={(event) => event.preventDefault()}
          className="fixed inset-x-0 bottom-0 z-[200] max-h-[85dvh] overflow-y-auto border-t border-border bg-surface px-5 pt-5 pb-[max(1.25rem,var(--sab,env(safe-area-inset-bottom)))] shadow-xl sm:px-8"
        >
          <div className="mx-auto flex w-full max-w-6xl flex-col gap-4 sm:flex-row sm:items-center sm:justify-between sm:gap-8">
            <div className="min-w-0 space-y-2">
              <DialogPrimitive.Title className="type-group-heading text-foreground">
                {translateUi("Usage analytics")}
              </DialogPrimitive.Title>
              <DialogPrimitive.Description className="type-body text-muted-foreground">
                {policy === 2
                  ? translateUi("Optional usage analytics are linked to your account. Allow enables account permission and this browser. Declining withdraws account permission across devices. Each other device still needs its own opt-in, and Do Not Track or Global Privacy Control always disables collection. Optional crash diagnostics include app version and sanitized code locations, without messages or URLs; reports are kept for up to 30 days. Change this in Settings → Privacy.")
                  : translateUi("Optional usage analytics are linked to your account. Allow enables account permission and this browser. Declining withdraws account permission across devices. Each other device still needs its own opt-in, and Do Not Track or Global Privacy Control always disables collection. Change this in Settings → Privacy.")}
              </DialogPrimitive.Description>
            </div>
            <div className="flex shrink-0 items-center gap-3 [&>button]:flex-1 sm:[&>button]:flex-none">
              <Button
                variant="outline"
                disabled={saving}
                onClick={() => void choose(false)}
                aria-label={translateUi("Decline telemetry")}
              >{translateUi("No Thanks")}</Button>
              <Button
                disabled={saving}
                onClick={() => void choose(true)}
                aria-label={translateUi("Accept telemetry")}
              >{translateUi("Allow")}</Button>
            </div>
          </div>
        </DialogPrimitive.Content>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  );
}
