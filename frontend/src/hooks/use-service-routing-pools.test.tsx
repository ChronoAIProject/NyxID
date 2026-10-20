import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useServiceRoutingPools } from "./use-service-routing-pools";
import type { KeyInfo } from "@/types/keys";
const mock = vi.hoisted(() => ({ get: vi.fn() }));
vi.mock("@/lib/api-client", () => ({ api: mock }));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (selector: (state: { user: { id: string } }) => unknown) =>
    selector({ user: { id: "me" } }),
}));
const pool = {
  id: "pool",
  user_id: "me",
  name: "Route",
  slug: "route",
  strategy: "priority",
  members: [],
  rr_counter: 0,
  is_active: true,
  created_at: "2026-01-01",
  updated_at: "2026-01-01",
};
const keys = [
  { id: "personal", credential_source: { type: "personal" } },
  {
    id: "admin",
    credential_source: {
      type: "org",
      org_id: "team",
      role: "admin",
      allowed: true,
    },
  },
  {
    id: "viewer",
    credential_source: {
      type: "org",
      org_id: "read-only",
      role: "viewer",
      allowed: false,
    },
  },
] as KeyInfo[];
function setup() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  const wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  return { client, wrapper };
}
beforeEach(() => {
  mock.get.mockReset();
  mock.get.mockResolvedValue({ pools: [pool] });
});
describe("service card pool inventory", () => {
  it("reads personal and manageable org pools without requesting restricted org inventory", async () => {
    const { result } = renderHook(() => useServiceRoutingPools(keys), setup());
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(mock.get.mock.calls.map(([path]) => path).sort()).toEqual([
      "/service-pools",
      "/service-pools?org_id=team",
    ]);
    expect(result.current.incomplete).toBe(true);
  });
  it("clears stale routing after a pool read loses access", async () => {
    const { client, wrapper } = setup();
    const { result } = renderHook(() => useServiceRoutingPools([keys[0]!]), {
      wrapper,
    });
    await waitFor(() => expect(result.current.pools).toHaveLength(1));
    mock.get.mockRejectedValue(new Error("Access removed"));
    await act(() => client.invalidateQueries({ queryKey: ["service-pools"] }));
    await waitFor(() => expect(result.current.incomplete).toBe(true));
    expect(result.current.pools).toEqual([]);
  });
});
