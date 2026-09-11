import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { describe, expect, it, vi } from "vitest";
import { useApproveAgentKeyLogin } from "./use-agent-key-login";

const { post } = vi.hoisted(() => ({ post: vi.fn() }));
vi.mock("@/lib/api-client", () => ({ api: { post }, apiClient: vi.fn() }));

function wrapper({ children }: PropsWithChildren) {
  return <QueryClientProvider client={new QueryClient()}>{children}</QueryClientProvider>;
}

describe("restricted device approval rollout", () => {
  it("posts only to the fail-closed route and never retries the account route on 404", async () => {
    post.mockRejectedValueOnce({ status: 404 });
    const { result } = renderHook(() => useApproveAgentKeyLogin("device"), { wrapper });
    await act(async () => {
      await expect(result.current.mutateAsync({ user_code: "ABCDEFGH", selection: {kind: "existing", api_key_id: "fixture"} })).rejects.toEqual({status: 404});
    });
    expect(post).toHaveBeenCalledExactlyOnceWith("/auth/device/approve-agent-key", {
      user_code: "ABCDEFGH", selection: {kind: "existing", api_key_id: "fixture"},
    });
  });
});
