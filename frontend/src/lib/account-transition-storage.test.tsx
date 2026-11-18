import { act, cleanup, render, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { AccountBoundary } from "@/components/account-boundary";
import { useAuthStore } from "@/stores/auth-store";
import { useAssistantDraftStore } from "@/stores/assistant-draft-store";
import { useAssistantContextStore } from "@/stores/assistant-context-store";
import { useAssistantWireLogStore } from "@/stores/assistant-wire-log-store";
import { useAssistantPinsStore } from "@/stores/assistant-pins-store";
import { useMachinePaneStore } from "@/stores/machine-pane-store";
import { useCmaSessionStore } from "@/stores/cma-session-store";
import { cmaSession } from "./assistant/cma-session.test-fixtures";
import { getAssistantIdentityUserId } from "./assistant/identity";
import { createAccountQueryScope } from "./account-query-cache";
import { installAccountSessionSync } from "./account-session-sync";
import type { User } from "@/types/api";

const person = (id: string) => ({ id, email: `${id}@example.test` }) as User;
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});
it.each(["B", null])(
  "removes mounted privacy covers and coherently publishes %s with every persistence write failing",
  async (next) => {
    localStorage.clear();
    useAuthStore.getState().setUser(person("A"));
    const scope = createAccountQueryScope({});
    const previous = scope.getClient();
    previous.setQueryData(["private"], "A private data");
    useAssistantDraftStore.getState().saveDraft("A", "draft", "A text");
    useAssistantContextStore.getState().recordScreen("A", "A screen");
    useAssistantWireLogStore.getState().setCaptureEnabled(true);
    useAssistantPinsStore.getState().toggle("A", "A pin");
    useMachinePaneStore.getState().setHidden("A chat", true);
    useCmaSessionStore
      .getState()
      .setMinimised(
        "A",
        `${cmaSession.connection.id}:${cmaSession.agentId}`,
        true,
      );
    const generation = useAuthStore.getState().accountGeneration;
    const verify = useAuthStore.getState().verifySession;
    const verification = vi.fn(verify);
    useAuthStore.setState({ verifySession: verification });
    const writes: string[] = [];
    vi.spyOn(localStorage, "setItem").mockImplementation((key) => {
      writes.push(key);
      throw new DOMException("quota", "QuotaExceededError");
    });
    vi.spyOn(localStorage, "removeItem").mockImplementation(() => {
      throw new DOMException("quota", "QuotaExceededError");
    });
    vi.stubGlobal(
      "BroadcastChannel",
      class {
        close() {}
        postMessage() {}
      },
    );
    let reply!: (value: Response) => void;
    vi.stubGlobal(
      "fetch",
      vi.fn(
        () =>
          new Promise<Response>((resolve) => {
            reply = resolve;
          }),
      ),
    );
    const stop = installAccountSessionSync();
    render(
      <AccountBoundary>
        <div>Account surface</div>
      </AccountBoundary>,
    );
    try {
      act(() =>
        window.dispatchEvent(
          new StorageEvent("storage", {
            key: "nyxid:account-session",
            newValue: JSON.stringify({
              userId: next,
              nonce: crypto.randomUUID(),
            }),
          }),
        ),
      );
      expect(document.documentElement).toHaveAttribute(
        "data-account-verifying",
      );
      await waitFor(() => expect(reply).toBeTypeOf("function"));
      await act(async () => {
        reply(
          next
            ? Response.json(person(next))
            : Response.json({}, { status: 401 }),
        );
        expect(await verification.mock.results[0]!.value).toBe(true);
      });
      await waitFor(() =>
        expect(document.documentElement).not.toHaveAttribute(
          "data-account-transition",
        ),
      );
      expect(document.documentElement).not.toHaveAttribute(
        "data-account-verifying",
      );
      expect(useAuthStore.getState().isIdentityVerifying).toBe(false);
      expect(useAuthStore.getState().user?.id ?? null).toBe(next);
      expect(getAssistantIdentityUserId()).toBe(next);
      expect(scope.getClient()).not.toBe(previous);
      expect(useAuthStore.getState().accountGeneration).toBe(generation + 1);
      expect(previous.getQueryData(["private"])).toBeUndefined();
      expect(useAssistantDraftStore.getState().drafts).toEqual({});
      expect(useAssistantContextStore.getState().lastScreen).toBeNull();
      expect(useAssistantWireLogStore.getState().entries).toEqual([]);
      expect(useAssistantWireLogStore.getState().captureEnabled).toBe(false);
      expect(useAssistantPinsStore.getState().ids).toEqual([]);
      expect(useMachinePaneStore.getState().hidden).toEqual([]);
      expect(useCmaSessionStore.getState().minimised).toEqual({});
      expect(writes).toContain("nyxid:account-session");
    } finally {
      stop();
      scope.dispose();
      useAuthStore.setState({ verifySession: verify });
    }
  },
);
