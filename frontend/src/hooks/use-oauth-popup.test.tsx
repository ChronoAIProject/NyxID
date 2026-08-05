import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useOAuthPopupStore } from "@/stores/oauth-popup-store";
import { useOAuthPopupReceiver } from "./use-oauth-popup";

const NONCE_A = "8e1fcf2a-e679-4da2-9f54-2d90cd5f0085";
const NONCE_B = "b9f7ae84-aea8-44ea-b8c6-9e9e446bb3dc";

class MockBroadcastChannel {
  static instances: MockBroadcastChannel[] = [];
  readonly name: string;
  readonly messages: unknown[] = [];
  onmessage: ((event: MessageEvent<unknown>) => void) | null = null;
  constructor(name: string) {
    this.name = name;
    MockBroadcastChannel.instances.push(this);
  }
  postMessage(message: unknown) {
    this.messages.push(message);
  }
  close() {}
  emit(message: unknown) {
    this.onmessage?.(new MessageEvent("message", { data: message }));
  }
}

describe("useOAuthPopupReceiver", () => {
  let queryClient: QueryClient;
  beforeEach(() => {
    MockBroadcastChannel.instances = [];
    vi.stubGlobal("BroadcastChannel", MockBroadcastChannel);
    useOAuthPopupStore.setState({
      attempt: {
        launchId: "launch-a",
        nonce: NONCE_A,
        keyId: "key-a",
        slug: "github",
        startedAt: 1,
      },
    });
    queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
  });
  afterEach(() => {
    queryClient.clear();
    vi.unstubAllGlobals();
  });

  function Wrapper({ children }: PropsWithChildren) {
    return (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );
  }

  it("treats result as a wakeup and never trusts message state", async () => {
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");
    const onViewResult = vi.fn(() => true);
    renderHook(
      () =>
        useOAuthPopupReceiver({
          launchId: "launch-a",
          onRetry: vi.fn(),
          onViewResult,
          onDismiss: vi.fn(),
        }),
      { wrapper: Wrapper },
    );
    act(() =>
      MockBroadcastChannel.instances[0]?.emit({
        type: "oauth_result",
        status: "complete",
      }),
    );
    await waitFor(() =>
      expect(invalidate).toHaveBeenCalledWith({ queryKey: ["keys"] }),
    );
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["keys", "key-a"] });
    expect(onViewResult).not.toHaveBeenCalled();
    expect(useOAuthPopupStore.getState().attempt).toMatchObject({
      launchId: "launch-a",
      nonce: NONCE_A,
      keyId: "key-a",
    });
  });

  it("transfers retry only on the old capability channel, then rotates it", async () => {
    const onRetry = vi.fn().mockResolvedValue({
      nextNonce: NONCE_B,
      url: `https://github.com/login/oauth/authorize?state=1cc_${NONCE_B}`,
    });
    renderHook(
      () =>
        useOAuthPopupReceiver({
          launchId: "launch-a",
          onRetry,
          onViewResult: vi.fn(() => true),
          onDismiss: vi.fn(),
        }),
      { wrapper: Wrapper },
    );
    const oldChannel = MockBroadcastChannel.instances[0];
    act(() => oldChannel?.emit({ type: "oauth_action", action: "retry" }));
    await waitFor(() => expect(onRetry).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(oldChannel?.messages).toHaveLength(1));
    expect(oldChannel?.messages[0]).toEqual({
      type: "oauth_retry",
      nextNonce: NONCE_B,
      url: `https://github.com/login/oauth/authorize?state=1cc_${NONCE_B}`,
    });
    expect(oldChannel?.messages[0]).not.toHaveProperty("nonce");
    expect(useOAuthPopupStore.getState().attempt?.nonce).toBe(NONCE_B);
  });

  it("runs only one retry while an attempt-generation request is in flight", async () => {
    let resolveRetry:
      | ((value: { nextNonce: string; url: string }) => void)
      | undefined;
    const onRetry = vi.fn(
      () =>
        new Promise<{ nextNonce: string; url: string }>((resolve) => {
          resolveRetry = resolve;
        }),
    );
    renderHook(
      () =>
        useOAuthPopupReceiver({
          launchId: "launch-a",
          onRetry,
          onViewResult: vi.fn(() => true),
          onDismiss: vi.fn(),
        }),
      { wrapper: Wrapper },
    );
    const channel = MockBroadcastChannel.instances[0];

    act(() => {
      channel?.emit({ type: "oauth_action", action: "retry" });
      channel?.emit({ type: "oauth_action", action: "retry" });
    });

    expect(onRetry).toHaveBeenCalledTimes(1);
    resolveRetry?.({
      nextNonce: NONCE_B,
      url: `https://github.com/login/oauth/authorize?state=1cc_${NONCE_B}`,
    });
    await waitFor(() => expect(channel?.messages).toHaveLength(1));
  });
});
