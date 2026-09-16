import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, expect, it, vi } from "vitest";
import {
  useAdminPlatformCredentials,
  useUpdatePlatformCredentials,
  useClearPlatformCredentials,
} from "./use-admin-platform-credentials";
import { useManagedOnboarding } from "./use-channel-managed";

const mock = vi.hoisted(() => ({
  get: vi.fn(),
  patch: vi.fn(),
  delete: vi.fn(),
}));
vi.mock("@/lib/api-client", () => ({ api: mock }));
function wrapperFactory() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
}
beforeEach(() => vi.clearAllMocks());
it("loads descriptor inventory and keeps managed bootstrap opt-in", async () => {
  mock.get.mockResolvedValue([]);
  const { result } = renderHook(() => useAdminPlatformCredentials(), {
    wrapper: wrapperFactory(),
  });
  await waitFor(() => expect(result.current.isSuccess).toBe(true));
  expect(mock.get).toHaveBeenCalledWith("/admin/platform-credentials");
  mock.get.mockClear();
  renderHook(() => useManagedOnboarding("whatsapp", false), {
    wrapper: wrapperFactory(),
  });
  expect(mock.get).not.toHaveBeenCalled();
});
it("sends explicit field clears, token rotation and provider deletion", async () => {
  const saved = {
    provider: "meta",
    label: "Meta",
    platform: "whatsapp",
    available: false,
    fields: [],
    setup_checklist: [],
    callback_url: null,
    webhook_verify_token: null,
    updated_at: null,
  };
  mock.patch.mockResolvedValue({ ...saved, future_field: true });
  mock.get.mockResolvedValue([saved]);
  mock.delete.mockResolvedValue(undefined);
  const { result } = renderHook(
    () => ({
      update: useUpdatePlatformCredentials("meta"),
      clear: useClearPlatformCredentials("meta"),
    }),
    { wrapper: wrapperFactory() },
  );
  await act(() =>
    result.current.update.mutateAsync({
      fields: { app_secret: null },
      regenerate_verify_token: true,
    }),
  );
  expect(mock.patch).toHaveBeenCalledWith("/admin/platform-credentials/meta", {
    fields: { app_secret: null },
    regenerate_verify_token: true,
  });
  await waitFor(() => expect(result.current.update.data).toEqual(saved));
  await act(() => result.current.clear.mutateAsync());
  expect(mock.delete).toHaveBeenCalledWith("/admin/platform-credentials/meta");
});
