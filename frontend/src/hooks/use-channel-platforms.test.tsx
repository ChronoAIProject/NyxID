import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, expect, it, vi } from "vitest";
import { useChannelPlatforms, useChannelPlatformViews } from "./use-channel-platforms";
import { platformFixtures } from "@/test/fixtures/channel-platforms";
const { get } = vi.hoisted(() => ({ get: vi.fn() }));
vi.mock("@/lib/api-client", () => ({ api: { get } }));
function wrapper() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return function Wrapper({ children }: PropsWithChildren) {
    return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
  };
}
beforeEach(() => vi.resetAllMocks());
it("fetches the complete descriptor catalog and shares the cached request", async () => {
  get.mockResolvedValue({ platforms: platformFixtures });
  const { result } = renderHook(() => ({ query: useChannelPlatforms(), views: useChannelPlatformViews() }), { wrapper: wrapper() });
  await waitFor(() => expect(result.current.query.isSuccess).toBe(true));
  expect(get).toHaveBeenCalledExactlyOnceWith("/channel-platforms");
  expect(result.current.query.data?.platforms).toEqual(platformFixtures);
  expect(result.current.views.getPlatform("x").managedOnly).toBe(true);
  expect(result.current.views.getPlatform("future").enabled).toBe(false);
});
it("exposes errors without inventing available platforms", async () => {
  get.mockRejectedValue(new Error("unavailable"));
  const { result } = renderHook(() => useChannelPlatformViews(), { wrapper: wrapper() });
  await waitFor(() => expect(result.current.isError).toBe(true));
  expect(result.current.platforms).toEqual({});
  expect(result.current.getPlatform("telegram").enabled).toBe(false);
});
