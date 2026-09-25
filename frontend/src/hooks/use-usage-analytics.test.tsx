import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, expect, it, vi } from "vitest";
import { api } from "@/lib/api-client";
import { useAnalyticsOptions } from "./use-usage-analytics";

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

it("searches organization names separately from personal account emails", async () => {
  vi.mocked(api.get).mockImplementation(async (path) => {
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
  expect(result.current.data).toEqual({
    options: [
      { id: "org-id", label: "Engineering", detail: "Organization" },
      {
        id: "person-id",
        label: "Engineer",
        detail: "engineering@example.test",
      },
    ],
    total: 71,
  });
});

it("does not fetch directory options until the filter is opened", () => {
  const { result } = renderHook(
    () => useAnalyticsOptions("actors", "", false),
    { wrapper },
  );
  expect(result.current.fetchStatus).toBe("idle");
  expect(api.get).not.toHaveBeenCalled();
});
