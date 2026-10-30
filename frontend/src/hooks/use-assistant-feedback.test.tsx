import { act, renderHook } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";
import { api } from "@/lib/api-client";
import { useAssistantFeedback, useReplyFeedback } from "./use-assistant-feedback";

vi.mock("@/lib/api-client", async (importOriginal) => ({
  ...await importOriginal<typeof import("@/lib/api-client")>(),
  api: { put: vi.fn(), delete: vi.fn() },
}));

describe("useAssistantFeedback", () => {
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={new QueryClient()}>
      {children}
    </QueryClientProvider>
  );

  it("changes a vote with a vote-only PUT and retains the saved comment for editing", async () => {
    vi.mocked(api.put).mockResolvedValue({ vote: "down" });
    const { result } = renderHook(() => useReplyFeedback({ scope: "conversation", parentId: "c", messageId: "m", initialVote: "up", initialComment: "Useful — explanation" }), { wrapper });
    await act(async () => { await result.current.toggle("down"); });
    expect(api.put).toHaveBeenLastCalledWith("/assistant/nyxagent/conversations/c/messages/m/feedback", { vote: "down" });
    expect(Object.keys(vi.mocked(api.put).mock.lastCall![1] as object)).toEqual(["vote"]);
    expect(result.current.comment).toBe("Useful — explanation");
  });

  it("clears a withdrawn comment before DELETE resolves and does not restore it over a re-vote", async () => {
    let finishDelete!: () => void;
    vi.mocked(api.delete).mockReturnValue(new Promise<void>((resolve) => { finishDelete = resolve; }));
    vi.mocked(api.put).mockResolvedValue({ vote: "down" });
    const { result } = renderHook(() => useReplyFeedback({ scope: "conversation", parentId: "c", messageId: "m", initialVote: "up", initialComment: "Useful — withdrawn" }), { wrapper });
    let withdrawal!: Promise<void>;
    act(() => { withdrawal = result.current.toggle("up"); });
    expect(result.current.comment).toBe("");
    await act(async () => { await result.current.toggle("down"); });
    expect(api.put).toHaveBeenLastCalledWith("/assistant/nyxagent/conversations/c/messages/m/feedback", { vote: "down" });
    await act(async () => { await result.current.send("New details"); });
    await act(async () => { finishDelete(); await withdrawal; });
    expect(result.current.comment).toBe("New details");
    expect(result.current.vote).toBe("down");
  });

  it("edits a group reply's saved comment through PUT without withdrawing", async () => {
    vi.mocked(api.put).mockResolvedValue({ vote: "down", comment: "Updated group comment" });
    const { result } = renderHook(() => useReplyFeedback({ scope: "group", parentId: "g", messageId: "m", initialVote: "down", initialComment: "Saved group comment" }), { wrapper });
    act(() => result.current.edit());
    expect(result.current.formOpen).toBe(true);
    expect(result.current.comment).toBe("Saved group comment");
    await act(async () => { await result.current.send("Updated group comment"); });
    expect(api.put).toHaveBeenLastCalledWith("/assistant/nyxagent/groups/g/messages/m/feedback", { vote: "down", comment: "Updated group comment" });
    expect(result.current.vote).toBe("down");
    expect(result.current.comment).toBe("Updated group comment");
    expect(result.current.formOpen).toBe(false);
  });

  it("uses PUT for a vote and DELETE to withdraw", async () => {
    vi.mocked(api.put).mockResolvedValue({ vote: "up" });
    vi.mocked(api.delete).mockResolvedValue(undefined);
    const { result } = renderHook(
      () => useAssistantFeedback("conversation", "c", "m"),
      { wrapper },
    );
    await act(async () => {
      await result.current.mutateAsync({ vote: "up", comment: "helpful" });
      await result.current.mutateAsync({});
    });
    expect(api.put).toHaveBeenCalledWith(
      "/assistant/nyxagent/conversations/c/messages/m/feedback",
      { vote: "up", comment: "helpful" },
    );
    expect(api.delete).toHaveBeenCalledWith(
      "/assistant/nyxagent/conversations/c/messages/m/feedback",
    );
  });

  it("clears pending state after a failed request for rollback callers", async () => {
    vi.mocked(api.put).mockRejectedValue(new Error("nope"));
    const { result } = renderHook(
      () => useAssistantFeedback("group", "g", "m"),
      { wrapper },
    );
    await expect(
      act(async () => {
        await result.current.mutateAsync({ vote: "down" });
      }),
    ).rejects.toThrow("nope");
    expect(result.current.isPending).toBe(false);
  });

  it("does not report success from a rejected mutation", async () => {
    vi.mocked(api.put).mockRejectedValue(new Error("offline"));
    const { result } = renderHook(
      () => useAssistantFeedback("conversation", "c", "m"),
      { wrapper },
    );
    await expect(
      act(async () => {
        await result.current.mutateAsync({ vote: "up" });
      }),
    ).rejects.toThrow("offline");
    expect(result.current.isPending).toBe(false);
  });

  it("invalidates the real history cache key after a successful vote", async () => {
    const client = new QueryClient();
    const invalidate = vi.spyOn(client, "invalidateQueries");
    const customWrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    );
    vi.mocked(api.put).mockResolvedValue({ vote: "up" });
    const { result } = renderHook(
      () => useAssistantFeedback("conversation", "c", "m"),
      { wrapper: customWrapper },
    );
    await act(async () => {
      await result.current.mutateAsync({ vote: "up" });
    });
    expect(invalidate).toHaveBeenCalledWith({
      queryKey: ["assistant", "nyxagent", undefined, "history", "c"],
    });
  });

  it("exposes mutation state for optimistic cache owners", () => {
    const { result } = renderHook(
      () => useAssistantFeedback("group", "g", "m"),
      { wrapper },
    );
    expect(result.current.isPending).toBe(false);
    expect(result.current.mutateAsync).toEqual(expect.any(Function));
  });
});
