import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { useAssistantFeedback } from "./use-assistant-feedback";
import { api } from "@/lib/api-client";

vi.mock("@/lib/api-client", () => ({ api: { put: vi.fn(), delete: vi.fn() } }));

describe("useAssistantFeedback", () => {
  it("uses PUT for a vote and DELETE to withdraw", async () => {
    vi.mocked(api.put).mockResolvedValue({ vote: "up" });
    vi.mocked(api.delete).mockResolvedValue(undefined);
    const { result } = renderHook(() =>
      useAssistantFeedback("conversation", "c", "m"),
    );
    await act(async () => {
      await result.current.mutateAsync({ vote: "up", comment: "helpful" });
    });
    await act(async () => {
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
    const { result } = renderHook(() =>
      useAssistantFeedback("group", "g", "m"),
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
    const { result } = renderHook(() =>
      useAssistantFeedback("conversation", "c", "m"),
    );
    await expect(
      act(async () => {
        await result.current.mutateAsync({ vote: "up" });
      }),
    ).rejects.toThrow("offline");
    expect(result.current.isPending).toBe(false);
  });
});
