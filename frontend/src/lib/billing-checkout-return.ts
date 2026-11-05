import { z } from "zod";

const KEY = "nyxid:billing-checkout-return:v1";
const MAX_AGE_MS = 2 * 60 * 60 * 1000;
const recordSchema = z.object({
  actorId: z.string().uuid(),
  ownerId: z.string().uuid().optional(),
  presentation: z.enum(["page", "panel"]),
  expiresAt: z.number().finite(),
});

/** Lago's configured return is /billing. Keep only navigation metadata in this
 * tab, bound to the signed-in person; never a payment or authorization decision. */
export function rememberBillingCheckout(actorId: string | undefined, ownerId: string | undefined, presentation: "page" | "panel") {
  if (!actorId) {
    if (ownerId) throw new Error("Sign in again before adding organization credits.");
    return;
  }
  try {
    sessionStorage.setItem(KEY, JSON.stringify(recordSchema.parse({ actorId, ownerId, presentation, expiresAt: Date.now() + MAX_AGE_MS })));
  } catch {
    if (ownerId) throw new Error("Allow session storage before continuing so payment returns to this organization's Billing view.");
  }
}

export function billingCheckoutReturn(actorId: string | undefined, search: Record<string, unknown>) {
  try {
    const raw = sessionStorage.getItem(KEY);
    if (!raw || !actorId) return undefined;
    const parsed = recordSchema.safeParse(JSON.parse(raw));
    if (!parsed.success || parsed.data.actorId !== actorId || parsed.data.expiresAt <= Date.now() || parsed.data.expiresAt > Date.now() + MAX_AGE_MS) return undefined;
    // Explicit navigation always wins. Only the provider's bare return consumes
    // the saved selection; it is then carried by normal URL/query-cache state.
    if (Object.values(search).some((value) => value !== undefined)) return undefined;
    const { ownerId, presentation } = parsed.data;
    if (presentation === "panel") return { to: "/assistant" as const, search: { panel: "billing" as const, panelOwnerId: ownerId } };
    if (ownerId) return { to: "/billing" as const, search: { ownerId } };
  } catch { /* Storage denial or stale data must never change authority. */ }
  return undefined;
}

export function consumeBillingCheckoutReturn(actorId: string | undefined, search: Record<string, unknown>) {
  const destination = billingCheckoutReturn(actorId, search);
  if (actorId) {
    try { sessionStorage.removeItem(KEY); } catch { /* Navigation metadata only. */ }
  }
  return destination;
}
