import { act, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { PropsWithChildren } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  assistantKeys,
  assistantKeysFor,
  directAssistantKeys,
  useAssistantEngine,
  useCreateConversation,
} from "@/hooks/use-assistant";
import { transitionAssistantIdentity } from "@/lib/assistant/identity";
import { assistantTransport } from "@/lib/assistant/transport";
import type { Conversation } from "@/types/assistant";

function harness() {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });
  const Wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
  return { queryClient, Wrapper };
}

function conversation(id: string): Conversation {
  return {
    id,
    title: id,
    created_at: "2026-08-11T00:00:00.000Z",
    last_message_at: "2026-08-11T00:00:00.000Z",
  };
}

afterEach(() => {
  vi.restoreAllMocks();
  transitionAssistantIdentity(null);
});

describe("assistant engine query ownership", () => {
  it("keeps the flag-off Aevatar keys unchanged and separates direct data", () => {
    expect(assistantKeys.conversations).toEqual(["assistant", "conversations"]);
    expect(assistantKeys.history("chatc-one")).toEqual([
      "assistant",
      "history",
      "chatc-one",
    ]);
    expect(directAssistantKeys.history("user-a", "direct-one")).toEqual([
      "assistant",
      "direct",
      "user-a",
      "history",
      "direct-one",
    ]);
    expect(
      assistantKeysFor("direct", "user-a").history("direct-one"),
    ).not.toEqual(assistantKeysFor("direct", "user-b").history("direct-one"));
  });

  it("removes old-engine queries on a flag flip", async () => {
    const { queryClient, Wrapper } = harness();
    const directKeys = assistantKeysFor("direct", "user-a");
    queryClient.setQueryData(assistantKeys.conversations, [
      conversation("chatc-old"),
    ]);
    queryClient.setQueryData(directKeys.conversations, [
      conversation("direct-current"),
    ]);
    const hook = renderHook(
      ({ engine }: { engine: "aevatar" | "direct" }) =>
        useAssistantEngine(engine),
      { wrapper: Wrapper, initialProps: { engine: "aevatar" } },
    );

    hook.rerender({ engine: "direct" });

    await waitFor(() =>
      expect(
        queryClient.getQueryData(assistantKeys.conversations),
      ).toBeUndefined(),
    );
    expect(queryClient.getQueryData(directKeys.conversations)).toEqual([
      conversation("direct-current"),
    ]);
  });

  it("invalidates direct queries on an identity transition", async () => {
    const { queryClient, Wrapper } = harness();
    const directKeys = assistantKeysFor("direct", "user-a");
    renderHook(() => useAssistantEngine("direct"), { wrapper: Wrapper });
    queryClient.setQueryData(directKeys.conversations, [
      conversation("direct-private"),
    ]);

    act(() => transitionAssistantIdentity("another-user"));

    await waitFor(() =>
      expect(
        queryClient.getQueryData(directKeys.conversations),
      ).toBeUndefined(),
    );
  });

  it("does not reuse an Aevatar pending create after switching to direct", async () => {
    const { Wrapper } = harness();
    let resolveAevatar: (value: Conversation) => void = () => undefined;
    let resolveDirect: (value: Conversation) => void = () => undefined;
    const createSpy = vi
      .spyOn(assistantTransport, "createConversation")
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            resolveAevatar = resolve;
          }),
      )
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            resolveDirect = resolve;
          }),
      );
    vi.spyOn(assistantTransport, "getHistory").mockRejectedValue(
      new Error("not materialized"),
    );
    vi.spyOn(assistantTransport, "listConversations").mockResolvedValue([]);
    const hook = renderHook(
      () => ({
        aevatar: useCreateConversation("aevatar"),
        direct: useCreateConversation("direct"),
      }),
      { wrapper: Wrapper },
    );

    let aevatarCreate!: Promise<Conversation>;
    let directCreate!: Promise<Conversation>;
    act(() => {
      aevatarCreate = hook.result.current.aevatar.mutateAsync();
      directCreate = hook.result.current.direct.mutateAsync();
    });
    await waitFor(() => expect(createSpy).toHaveBeenCalledTimes(2));

    resolveAevatar(conversation("chatc-aevatar"));
    resolveDirect(conversation("direct-new"));
    await act(async () => {
      await Promise.all([aevatarCreate, directCreate]);
    });
  });
});
