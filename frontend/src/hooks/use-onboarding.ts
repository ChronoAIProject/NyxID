import { useMutation } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import type { OnboardingState, User } from "@/types/api";

/**
 * Whether the signed-in user should be shown the onboarding takeover in
 * place of the normal dashboard.
 *
 * - `loading` — auth / `GET /users/me` has not settled; render nothing and
 *   decide nothing (deciding on un-loaded data is the flash/loop bug).
 * - `show`    — render the onboarding takeover over the dashboard.
 * - `hidden`  — nothing to do; render the dashboard normally.
 */
export type OnboardingGate =
  | { readonly status: "loading" }
  | { readonly status: "hidden" }
  | { readonly status: "show" };

interface OnboardingCheckContext {
  readonly user: User;
}

/**
 * One onboarding trigger. `evaluate` returns true if this flow still needs
 * to run. Add new first-run flows by appending a check here — this array is
 * the single extension point.
 */
interface OnboardingCheck {
  readonly id: string;
  evaluate(ctx: OnboardingCheckContext): boolean;
}

const CHECKS: readonly OnboardingCheck[] = [
  {
    id: "ai-services-wizard",
    evaluate({ user }) {
      // Fail open: older backends omit `profile_config` entirely. Treat
      // "unknown" as done — never trap a user behind a flag we can't read.
      if (!user.profile_config) return false;
      return !user.profile_config.onboarding.ai_services_completed_at;
    },
  },
];

/**
 * Decides whether to show the onboarding takeover. Mount once in
 * `DashboardLayout` — it gates every authenticated route, rendering the
 * wizard in place of the dashboard chrome rather than redirecting to a
 * separate route. Gated on auth + `GET /users/me` settling so it never
 * flashes the wrong thing.
 */
export function useShouldShowOnboarding(): OnboardingGate {
  const user = useAuthStore((s) => s.user);
  const isLoading = useAuthStore((s) => s.isLoading);
  const isAuthenticated = useAuthStore((s) => s.isAuthenticated);

  // Wait for auth + `GET /users/me` to settle before deciding anything.
  if (isLoading) return { status: "loading" };
  // Unauthenticated is the router's auth guards' problem, not ours.
  if (!isAuthenticated || !user) return { status: "hidden" };

  const needsOnboarding = CHECKS.some((check) => check.evaluate({ user }));
  return needsOnboarding ? { status: "show" } : { status: "hidden" };
}

/**
 * Marks a first-run onboarding flow as completed (or skipped) for the
 * current user. The caller should refresh the auth-store user afterward
 * (e.g. `checkAuth()`) so `useShouldShowOnboarding` sees the new flag and
 * the layout swaps the takeover for the real dashboard.
 */
export function useCompleteOnboarding() {
  return useMutation({
    mutationFn: (vars: { key: string }) =>
      api.post<OnboardingState>("/users/me/onboarding/complete", vars),
  });
}
