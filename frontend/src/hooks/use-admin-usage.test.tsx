import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { api } from "@/lib/api-client";
import { usageFixture } from "@/test/admin-usage-fixture";
import { normalizeAdminUsageSearch } from "@/schemas/admin-usage";
import { useAuthStore } from "@/stores/auth-store";
import {
  adminUsagePath,
  adminUsageQueryOptions,
  useAdminUsage,
  usePreloadAdminUsageDetails,
} from "./use-admin-usage";
vi.mock("@/lib/api-client", () => ({ api: { get: vi.fn() } }));
const client = new QueryClient({
  defaultOptions: { queries: { retry: false } },
});
const originalAuth = useAuthStore.getState();
function wrapper({ children }: PropsWithChildren) {
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}
beforeEach(() => {
  vi.clearAllMocks();
  client.clear();
});
afterEach(() => useAuthStore.setState(originalAuth));
it("builds a validated query with every control and parses the receipt", async () => {
  const params = normalizeAdminUsageSearch({
    period: "7d",
    user: usageFixture().ranking[0]!.user.id,
    service: "llm-example",
    sort: "quantity",
    metric: "images",
    page: 2,
    per_page: 50,
  });
  vi.mocked(api.get).mockResolvedValue(usageFixture());
  const { result } = renderHook(() => useAdminUsage(params), { wrapper });
  await waitFor(() => expect(result.current.isSuccess).toBe(true));
  expect(api.get).toHaveBeenCalledWith(adminUsagePath(params));
  expect(adminUsagePath(params)).toContain("metric=images&page=2&per_page=50");
  expect(result.current.data?.ranking[0]?.user.display_name).toBe("Alice");
});
it("custom ranges exclude period and invalid ranges never fetch", async () => {
  const custom = normalizeAdminUsageSearch({
    period: "custom",
    from: "2026-09-18T00:00:00Z",
    to: "2026-09-19T00:00:00Z",
  });
  expect(adminUsagePath(custom)).not.toContain("period=");
  expect(adminUsagePath(custom)).toContain("from=2026");
  const { result } = renderHook(
    () => useAdminUsage({ ...custom, to: undefined }),
    { wrapper },
  );
  expect(result.current.fetchStatus).toBe("idle");
  expect(api.get).not.toHaveBeenCalled();
});
it("rejects a malformed response instead of showing invented zero totals", async () => {
  vi.mocked(api.get).mockResolvedValue({ totals: { requests: "unknown" } });
  const { result } = renderHook(
    () => useAdminUsage(normalizeAdminUsageSearch({})),
    { wrapper },
  );
  await waitFor(() => expect(result.current.isError).toBe(true));
});
it("preloads every visible user's services with a reusable five-minute cache", async () => {
  const data = usageFixture();
  const secondUser = "33333333-3333-4333-8333-333333333333";
  data.ranking.push({
    ...data.ranking[0]!,
    user: { ...data.ranking[0]!.user, id: secondUser },
  });
  useAuthStore.setState({
    user: { ...originalAuth.user!, id: "admin-1" },
  });
  vi.mocked(api.get).mockResolvedValue(usageFixture());
  const params = {
    ...normalizeAdminUsageSearch({
      period: "custom",
      from: data.window.from,
      to: data.window.to,
    }),
    services: [],
    actors: [],
    owners: [],
  };
  const { unmount } = renderHook(
    () => usePreloadAdminUsageDetails(data, params),
    { wrapper },
  );
  await waitFor(() => expect(api.get).toHaveBeenCalledTimes(2));
  const first = { ...params, user: data.ranking[0]!.user.id };
  expect(adminUsageQueryOptions(first, "admin-1")).toMatchObject({
    staleTime: 5 * 60_000,
    gcTime: 10 * 60_000,
  });
  expect(
    client.getQueryData(adminUsageQueryOptions(first, "admin-1").queryKey),
  ).toBeDefined();
  expect(
    client.getQueryState(adminUsageQueryOptions(first, "admin-1").queryKey)
      ?.isInvalidated,
  ).toBe(false);
  const { result } = renderHook(() => useAdminUsage(first), { wrapper });
  await waitFor(() => expect(result.current.isSuccess).toBe(true));
  expect(api.get).toHaveBeenCalledTimes(2);
  unmount();
});
