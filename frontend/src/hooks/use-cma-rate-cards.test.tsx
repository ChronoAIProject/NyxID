import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useCmaRateCards, usePublishCmaRateCard } from "./use-cma-rate-cards";
import { cmaRatesSchema } from "@/schemas/cma-rate-cards";
const mock = vi.hoisted(() => ({ get: vi.fn(), post: vi.fn() }));
vi.mock("@/lib/api-client", () => ({ api: mock }));
const rates = {
  sandbox_cpu_millis_seconds: "0.00000001",
  sandbox_memory_mib_seconds: "0.000000001",
  sandbox_disk_gib_seconds: "0.00000004",
};
const card = {
  id: "one",
  version: 1,
  effective_from: "2026-01-01T00:00:00Z",
  rates,
  sync_status: "pending",
  created_at: "2026-01-01T00:00:00Z",
  created_by: null,
};
function wrapper() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
}
beforeEach(() => vi.clearAllMocks());
describe("CMA rate card API", () => {
  it("loads history and publishes a version without rounding exact rates", async () => {
    mock.get.mockResolvedValue([card]);
    mock.post.mockResolvedValue({ ...card, version: 2 });
    const read = renderHook(() => useCmaRateCards(), { wrapper: wrapper() });
    await waitFor(() => expect(read.result.current.isSuccess).toBe(true));
    expect(mock.get).toHaveBeenCalledWith("/admin/billing/cma-rate-cards");
    const write = renderHook(() => usePublishCmaRateCard(), {
      wrapper: wrapper(),
    });
    const input = {
      expected_version: 1,
      effective_from: "2027-01-01T00:00:00Z",
      rates,
    };
    await act(() => write.result.current.mutateAsync(input));
    expect(mock.post).toHaveBeenCalledWith(
      "/admin/billing/cma-rate-cards",
      input,
    );
  });
  it("rejects a malformed server contract", async () => {
    mock.get.mockResolvedValue([{ ...card, rates: {} }]);
    const read = renderHook(() => useCmaRateCards(), { wrapper: wrapper() });
    await waitFor(() => expect(read.result.current.isError).toBe(true));
  });
  it.each([
    "0",
    "-1",
    "NaN",
    "1e-8",
    "0.0000000000001",
    "1000000.000000000001",
  ])("rejects invalid prepaid rate %s", (value) => {
    expect(
      cmaRatesSchema.safeParse({ ...rates, sandbox_cpu_millis_seconds: value })
        .success,
    ).toBe(false);
  });
});
