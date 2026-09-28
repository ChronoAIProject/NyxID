import { lazy, Suspense, useEffect } from "react";
import { useRouterState } from "@tanstack/react-router";
import {
  currentCreditsActor,
  isCreditsDialogSuppressed,
  type CreditsPayer,
} from "@/lib/credits-denial";
import { useAuthStore } from "@/stores/auth-store";
import { useCreditsDenialStore } from "@/stores/credits-denial-store";
import { isBillingAvailable } from "@/types/api";

const CreditsDeniedDialog = lazy(() => import("./credits-denied-dialog"));

const PREVIEW_PAYERS: Record<string, CreditsPayer> = {
  self: "self",
  unknown: "unknown",
  org: { org: { id: "preview-org", name: "Acme Research" } },
  unavailable: "self",
};

/** Root-level presenter for `useCreditsDenialStore`; mounted once. */
export function CreditsDeniedHost() {
  const user = useAuthStore((state) => state.user);
  const current = useCreditsDenialStore((state) => state.current);
  const dismiss = useCreditsDenialStore((state) => state.dismiss);
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const suppressed = isCreditsDialogSuppressed(pathname);
  const searchStr = useRouterState({ select: (s) => s.location.searchStr });
  const previewUnavailable =
    import.meta.env.DEV &&
    new URLSearchParams(searchStr).get("credits-preview") === "unavailable";

  useEffect(() => {
    if (current && suppressed) dismiss();
  }, [current, suppressed, dismiss]);

  // DEV-only design check: `?credits-preview=self|unknown|org|unavailable`.
  // `import.meta.env.DEV` is statically false in production builds.
  const userId = user?.id;
  useEffect(() => {
    if (!import.meta.env.DEV || !userId) return;
    const variant = new URLSearchParams(window.location.search).get(
      "credits-preview",
    );
    const payer = variant ? PREVIEW_PAYERS[variant] : undefined;
    if (!payer) return;
    useCreditsDenialStore.getState().notify({
      key: `preview:${variant}`,
      payer,
      actorId: currentCreditsActor(),
    });
  }, [userId]);

  if (!user || !current || suppressed) return null;
  return (
    <Suspense fallback={null}>
      <CreditsDeniedDialog
        payer={current.payer}
        billingAvailable={isBillingAvailable(user) && !previewUnavailable}
        onDismiss={dismiss}
      />
    </Suspense>
  );
}
