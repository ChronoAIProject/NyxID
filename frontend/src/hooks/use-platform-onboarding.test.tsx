import { act, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { PropsWithChildren } from "react";
import { beforeEach, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import type { PlatformProfile } from "@/schemas/platform-onboarding";
import { useAdminPlatformAction } from "./use-platform-onboarding";
const { request } = vi.hoisted(() => ({ request: vi.fn() }));
vi.mock("@/lib/api-client", () => ({ apiClient: request }));
const profile: PlatformProfile = {
  id: "11111111-1111-4111-8111-111111111111",
  channel_bot_id: "22222222-2222-4222-8222-222222222222",
  manager_bot_id: "channel:slack:bot",
  execution_profile: "platform_onboarding",
  generation: 3,
  enabled: false,
  funding_owner_user_id: "33333333-3333-4333-8333-333333333333",
  deleted_at: null,
};
beforeEach(() => {
  vi.clearAllMocks();
  useAuthStore.setState({
    user: { id: "admin", role: "admin" } as User,
    isAuthenticated: true,
  });
});
function setup() {
  const client = new QueryClient({
    defaultOptions: { mutations: { retry: false } },
  });
  const wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  return { client, ...renderHook(() => useAdminPlatformAction(), { wrapper }) };
}
it.each([true, false])(
  "erases replacement fields from real mutation cache and request body after settlement (success=%s)",
  async (success) => {
    let settle!: (value: unknown) => void;
    request.mockImplementation(
      () =>
        new Promise((resolve, reject) => {
          settle = success ? resolve : reject;
        }),
    );
    const { client, result, unmount } = setup();
    const input = {
      action: "repair" as const,
      profile,
      fields: { bot_token: "ephemeral-replacement" },
    };
    act(() => result.current.mutate(input));
    await waitFor(() => expect(request).toHaveBeenCalledOnce());
    expect(
      JSON.stringify(client.getMutationCache().getAll()[0]!.state.variables),
    ).toContain("ephemeral-replacement");
    await act(async () => settle(success ? profile : new Error("conflict")));
    await waitFor(() => expect(result.current.isPending).toBe(false));
    expect(
      JSON.stringify(
        client
          .getMutationCache()
          .getAll()
          .map((m) => m.state.variables),
      ),
    ).not.toContain("ephemeral-replacement");
    expect(JSON.stringify(request.mock.calls[0]![1].body)).not.toContain(
      "ephemeral-replacement",
    );
    unmount();
    await waitFor(() =>
      expect(client.getMutationCache().getAll()).toHaveLength(0),
    );
    client.clear();
  },
);
it("scrubs pending credentials on unmount", async () => {
  request.mockImplementation(() => new Promise(() => {}));
  const { client, result, unmount } = setup();
  act(() =>
    result.current.mutate({
      action: "repair",
      profile,
      fields: { bot_token: "pending-secret" },
    }),
  );
  await waitFor(() => expect(request).toHaveBeenCalledOnce());
  unmount();
  expect(
    JSON.stringify(
      client
        .getMutationCache()
        .getAll()
        .map((m) => m.state.variables),
    ),
  ).not.toContain("pending-secret");
  expect(JSON.stringify(request.mock.calls[0]![1].body)).not.toContain(
    "pending-secret",
  );
  client.clear();
});
