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

it("searches people, organizations, and service accounts by name", async () => {
  vi.mocked(api.get).mockImplementation(async (path) => {
    if (path.startsWith("/admin/service-accounts?"))
      return {
        service_accounts: [{ id: "sa-id", name: "Heca Engineering worker" }],
        total: 1,
      };
    const org = path.includes("user_type=org");
    return {
      users: [
        {
          id: org ? "org-id" : "person-id",
          display_name: org ? "Engineering" : "Engineer",
          email: org ? "generated@org.test" : "engineering@example.test",
        },
      ],
      total: org ? 1 : 70,
    };
  });
  const { result } = renderHook(
    () => useAnalyticsOptions("owners", "engineering", true),
    { wrapper },
  );
  await waitFor(() => expect(result.current.isSuccess).toBe(true));
  expect(api.get).toHaveBeenCalledWith(
    "/admin/users?page=1&per_page=50&user_type=org&search=engineering",
  );
  expect(api.get).toHaveBeenCalledWith(
    "/admin/users?page=1&per_page=50&user_type=person&search=engineering",
  );
  expect(api.get).toHaveBeenCalledWith(
    "/admin/service-accounts?page=1&per_page=50&search=engineering",
  );
  expect(result.current.data).toEqual({
    options: [
      { id: "org-id", label: "Engineering", detail: "Organization" },
      {
        id: "person-id",
        label: "Engineer",
        detail: "engineering@example.test",
      },
      {
        id: "sa-id",
        label: "Heca Engineering worker",
        detail: "Service account",
      },
    ],
    total: 72,
  });
});

it("resolves saved service-account filter names after reload", async () => {
  vi.mocked(api.get).mockImplementation(async (path) => {
    if (path === "/admin/users/sa-id")
      throw new ApiError(404, {
        error: "not_found",
        error_code: 1000,
        message: "User not found",
      });
    if (path === "/admin/service-accounts/sa-id")
      return { name: "Heca production worker" };
    throw new Error(`Unexpected request: ${path}`);
  });
  const { result } = renderHook(
    () =>
      useAnalyticsLabels({
        ...EMPTY_FILTERS,
        actors: ["sa-id"],
        owners: ["sa-id"],
      }),
    { wrapper },
  );
  await waitFor(() =>
    expect(result.current.actors["sa-id"]).toBe("Heca production worker"),
  );
  expect(result.current.owners["sa-id"]).toBe("Heca production worker");
  expect(api.get).toHaveBeenCalledTimes(2);
});

it("keeps the UUID when both identity records are missing", async () => {
  vi.mocked(api.get).mockRejectedValue(
    new ApiError(404, {
      error: "not_found",
      error_code: 1000,
      message: "Identity not found",
    }),
  );
  const { result } = renderHook(
    () =>
      useAnalyticsLabels({
        ...EMPTY_FILTERS,
        actors: ["deleted-id"],
      }),
    { wrapper },
  );
  await waitFor(() =>
    expect(api.get).toHaveBeenCalledWith("/admin/service-accounts/deleted-id"),
  );
  expect(result.current.actors["deleted-id"]).toBe("deleted-id");
});

it("does not fall back to service accounts on a directory authorization failure", async () => {
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
    expect(api.get).toHaveBeenCalledWith("/admin/users/person-id"),
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
