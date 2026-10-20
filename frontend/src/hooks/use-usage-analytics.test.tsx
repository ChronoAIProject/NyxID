import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, expect, it, vi } from "vitest";
import { api, ApiError } from "@/lib/api-client";
import { EMPTY_FILTERS } from "@/lib/usage-analytics";
import { useAnalyticsLabels, useAnalyticsOptions } from "./use-usage-analytics";

vi.mock("@/lib/api-client", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/api-client")>()),
  api: { get: vi.fn() },
}));
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

it("searches identities through the usage-scoped lookup", async () => {
  const options = [
    { id: "org-id", label: "Engineering", detail: "Organization" },
    { id: "person-id", label: "Engineer", detail: "engineering@example.test" },
    { id: "sa-id", label: "Heca Engineering worker", detail: "Service account" },
  ];
  vi.mocked(api.get).mockResolvedValue({ options, total: 72 });
  const { result } = renderHook(
    () => useAnalyticsOptions("owners", "engineering", true),
    { wrapper },
  );
  await waitFor(() => expect(result.current.isSuccess).toBe(true));
  expect(api.get).toHaveBeenCalledTimes(1);
  expect(api.get).toHaveBeenCalledWith(
    "/admin/usage/identities?kind=owners&search=engineering",
  );
  expect(result.current.data).toEqual({ options, total: 72 });
});

it("resolves saved filter names after reload in one request", async () => {
  vi.mocked(api.get).mockImplementation(async (path) => {
    if (path === "/admin/usage/identities?ids=person-id%2Csa-id")
      return {
        options: [
          { id: "sa-id", label: "Heca production worker", detail: "Service account" },
          { id: "person-id", label: "Engineer", detail: "e@example.test" },
        ],
        total: 2,
      };
    throw new Error(`Unexpected request: ${path}`);
  });
  const { result } = renderHook(
    () =>
      useAnalyticsLabels({
        ...EMPTY_FILTERS,
        actors: ["sa-id"],
        owners: ["sa-id", "person-id"],
      }),
    { wrapper },
  );
  await waitFor(() =>
    expect(result.current.actors["sa-id"]).toBe("Heca production worker"),
  );
  expect(result.current.owners["sa-id"]).toBe("Heca production worker");
  expect(result.current.owners["person-id"]).toBe("Engineer");
  expect(api.get).toHaveBeenCalledTimes(1);
});

it("labels more than 100 saved identities in batches", async () => {
  const actors = Array.from({ length: 150 }, (_, i) => `id-${String(i).padStart(3, "0")}`);
  vi.mocked(api.get).mockImplementation(async (path) => {
    const ids = new URLSearchParams(path.split("?")[1]).get("ids")!.split(",");
    expect(ids.length).toBeLessThanOrEqual(100);
    return {
      options: ids.map((id) => ({ id, label: `Label ${id}` })),
      total: ids.length,
    };
  });
  const { result } = renderHook(
    () => useAnalyticsLabels({ ...EMPTY_FILTERS, actors }),
    { wrapper },
  );
  await waitFor(() =>
    expect(result.current.actors["id-149"]).toBe("Label id-149"),
  );
  expect(result.current.actors["id-000"]).toBe("Label id-000");
  expect(api.get).toHaveBeenCalledTimes(2);
});

it("keeps the UUID when an identity no longer exists", async () => {
  vi.mocked(api.get).mockResolvedValue({ options: [], total: 0 });
  const { result } = renderHook(
    () =>
      useAnalyticsLabels({
        ...EMPTY_FILTERS,
        actors: ["deleted-id"],
      }),
    { wrapper },
  );
  await waitFor(() =>
    expect(api.get).toHaveBeenCalledWith("/admin/usage/identities?ids=deleted-id"),
  );
  expect(result.current.actors["deleted-id"]).toBe("deleted-id");
});

it("keeps the UUID when the lookup is refused", async () => {
  vi.mocked(api.get).mockRejectedValue(
    new ApiError(403, {
      error: "forbidden",
      error_code: 1000,
      message: "Forbidden",
    }),
  );
  const { result } = renderHook(
    () =>
      useAnalyticsLabels({
        ...EMPTY_FILTERS,
        actors: ["person-id"],
      }),
    { wrapper },
  );
  await waitFor(() =>
    expect(api.get).toHaveBeenCalledWith("/admin/usage/identities?ids=person-id"),
  );
  expect(result.current.actors["person-id"]).toBe("person-id");
  expect(api.get).toHaveBeenCalledTimes(1);
});

it("does not fetch directory options until the filter is opened", () => {
  const { result } = renderHook(
    () => useAnalyticsOptions("actors", "", false),
    { wrapper },
  );
  expect(result.current.fetchStatus).toBe("idle");
  expect(api.get).not.toHaveBeenCalled();
});
