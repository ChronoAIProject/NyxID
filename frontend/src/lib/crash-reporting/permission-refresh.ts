import type { QueryClient } from "@tanstack/react-query";
import { useAuthStore } from "@/stores/auth-store";
import { useConsentStore } from "@/stores/consent-store";
import type { PublicConfig } from "@/types/api";
import { invalidateCrashPermission, refreshCrashPermission } from "./reporter";

/** Refresh discovery only when its authority inputs change, using cached config. */
export function subscribeCrashPermission(client: QueryClient): () => void {
  function inputs() {
    const device = useConsentStore.getState();
    return [
      useAuthStore.getState().user?.id,
      device.enabled,
      device.policy,
      device.accountId,
      device.revision,
      client.getQueryData<PublicConfig>(["public-config"])?.crash_diagnostics_offered === true,
    ] as const;
  }
  let previous: ReturnType<typeof inputs> | undefined;
  function update() {
    const next = inputs();
    const last = previous;
    if (last && next.every((value, index) => value === last[index])) return;
    previous = next;
    void refreshCrashPermission(next[5]);
  }
  const unsubscribeAuth = useAuthStore.subscribe(update);
  const unsubscribeConsent = useConsentStore.subscribe(update);
  const unsubscribeConfig = client.getQueryCache().subscribe((event) => {
    if (event.query.queryKey.length === 1 && event.query.queryKey[0] === "public-config")
      update();
  });
  update();
  return () => {
    unsubscribeAuth();
    unsubscribeConsent();
    unsubscribeConfig();
    invalidateCrashPermission();
  };
}
