import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, expect, it, vi } from "vitest";
import { api } from "@/lib/api-client";
import { usageFixture } from "@/test/admin-usage-fixture";
import { normalizeAdminUsageSearch } from "@/schemas/admin-usage";
import { adminUsagePath, useAdminUsage } from "./use-admin-usage";
vi.mock("@/lib/api-client", () => ({ api: { get: vi.fn() } }));
function wrapper({ children }: PropsWithChildren) {
  return (
    <QueryClientProvider
      client={
        new QueryClient({ defaultOptions: { queries: { retry: false } } })
      }
    >
      {children}
    </QueryClientProvider>
  );
}
beforeEach(() => vi.clearAllMocks());
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
