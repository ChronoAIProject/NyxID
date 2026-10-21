import { act, renderHook } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { afterEach, expect, it, vi } from "vitest";
import { nyxBotApi } from "@/lib/assistant/nyxbot-api";
import { nyxBotQueryKeys, useConnectNyxBotChannel } from "./use-nyxbot-agents";

const client = new QueryClient({ defaultOptions: { mutations: { retry: false } } });
function wrapper({ children }: { children: ReactNode }) {
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}
afterEach(() => { client.clear(); vi.restoreAllMocks(); });

it("invalidates channels and agents after a rejected connect", async () => {
  const key = nyxBotQueryKeys.channels(undefined);
  client.setQueryData(key, []);
  vi.spyOn(nyxBotApi, "connectChannel").mockRejectedValue(new Error("Gateway refused"));
  const invalidate = vi.spyOn(client, "invalidateQueries");
  const { result, unmount } = renderHook(useConnectNyxBotChannel, { wrapper });
  await act(async () => {
    await expect(result.current.mutateAsync({ botId: "bot", agentId: "agent" })).rejects.toThrow("Gateway refused");
  });
  expect(invalidate).toHaveBeenCalledWith({ queryKey: key });
  expect(invalidate).toHaveBeenCalledWith({ queryKey: nyxBotQueryKeys.agents(undefined) });
  expect(client.getQueryState(key)?.isInvalidated).toBe(true);
  unmount();
});
