import { act, renderHook } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { expect, it, vi } from "vitest";
import { ChannelConnectLinkContext } from "./use-channel-connect-link";
import { useCreateChannelBot } from "./use-channel-bots";
import { api } from "@/lib/api-client";

vi.mock("@/lib/api-client", () => ({
  api: { post: vi.fn() },
  apiClient: vi.fn(),
}));

it.each([false, true])(
  "routes manual setup through the hosted token only inside its context (%s)",
  async (hosted) => {
    vi.mocked(api.post).mockResolvedValue({ id: "bot" });
    const client = new QueryClient();
    const wrapper = ({ children }: { children: React.ReactNode }) => (
      <QueryClientProvider client={client}>
        <ChannelConnectLinkContext.Provider
          value={
            hosted
              ? { id: "link", token: "nyx_bcl_test", refresh: vi.fn() }
              : null
          }
        >
          {children}
        </ChannelConnectLinkContext.Provider>
      </QueryClientProvider>
    );
    const hook = renderHook(useCreateChannelBot, { wrapper });
    const input = {
      platform: "telegram",
      label: "Support",
      bot_token: "bot-credential",
    };
    await act(() => hook.result.current.mutateAsync(input));
    expect(api.post).toHaveBeenLastCalledWith(
      hosted ? "/channel-connect-links/complete" : "/channel-bots",
      hosted ? { ...input, token: "nyx_bcl_test" } : input,
    );
  },
);
