import { act, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { afterEach, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import { invalidateAccountEvidence, settleAccountEvidence } from "@/lib/assistant/identity";
import { useSaveSecurityPreferences } from "./use-security-preferences";
import { useServiceView } from "./use-service-view";
import { DEFAULT_SERVICE_FILTERS } from "@/schemas/service-view";
import type { User } from "@/types/api";
const { write } = vi.hoisted(() => ({ write: vi.fn() }));
vi.mock("@/lib/api-client", async (original) => ({
  ...await original<typeof import("@/lib/api-client")>(), api: { put: write }, apiClient: write,
}));
afterEach(() => settleAccountEvidence());
it.each(["security", "default-view", "saved-views"])("preserves an accepted %s save without publishing a superseded profile", async (kind) => {
  const confirmed = { id: "A", email: "a@example.test", display_name: null, avatar_url: null, email_verified: true, mfa_enabled: false, is_admin: false, is_active: true, created_at: "2026-01-01", profile_config: { onboarding: { ai_services_completed_at: null }, security: { skip_mfa_for_approved_sign_ins: false }, services_view: DEFAULT_SERVICE_FILTERS, service_views: { views: [], default_id: null } } } as User;
  useAuthStore.getState().setUser(confirmed);
  const client = new QueryClient({ defaultOptions: { mutations: { retry: false } } });
  client.setQueryData(["user", "me"], confirmed);
  let release!: () => void;
  write.mockImplementation((_path, body) => new Promise(resolve => { release = () => resolve(kind === "security" ? { skip_mfa_for_approved_sign_ins: true } : body); }));
  const security = renderHook(useSaveSecurityPreferences, { wrapper: ({ children }: { children: ReactNode }) => <QueryClientProvider client={client}>{children}</QueryClientProvider> });
  const view = renderHook(useServiceView, { wrapper: ({ children }: { children: ReactNode }) => <QueryClientProvider client={client}>{children}</QueryClientProvider> });
  const saved = vi.fn();
  act(() => {
    if (kind === "security") security.result.current.mutate({ accountId: "A", skip_mfa_for_approved_sign_ins: true }, { onSuccess: saved });
    else if (kind === "default-view") view.result.current.saveDefault();
    else view.result.current.saveNewView("Accepted view", saved);
  });
  await waitFor(() => expect(release).toBeTypeOf("function"));
  act(() => { invalidateAccountEvidence(); settleAccountEvidence(); });
  await act(async () => release());
  if (kind === "security") await waitFor(() => expect(security.result.current.isSuccess).toBe(true));
  else await waitFor(() => expect(view.result.current.isSaving).toBe(false));
  if (kind !== "default-view") expect(saved).toHaveBeenCalledOnce();
  expect(useAuthStore.getState().user).toBe(confirmed);
  expect(client.getQueryData(["user", "me"])).toEqual(confirmed);
  security.unmount(); view.unmount(); client.clear();
});
