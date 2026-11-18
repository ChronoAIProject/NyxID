import { expect, it, vi } from "vitest";
import { runChatStream } from "./chat-stream-orchestrator";
import { createChatActorProjection } from "./chat-actor-state";
import { createDraftChatSession } from "./chat-session-state";
import { AccountChangedError } from "./identity";
import { captureNyxbotError } from "./error-telemetry";
vi.mock("./chat-api", async (original) => ({
  ...await original<typeof import("./chat-api")>(),
  sendChatCommand: vi.fn(async () => { throw new AccountChangedError(); }),
}));
vi.mock("./error-telemetry", async (original) => ({
  ...await original<typeof import("./error-telemetry")>(),
  captureNyxbotError: vi.fn(),
}));
it("restores the pre-send draft silently on a predispatch account interruption", async () => {
  const base = createDraftChatSession();
  const initialProjection = createChatActorProjection();
  const updateEntry = vi.fn();
  await expect(runChatStream({
    base, initialProjection, controller: new AbortController(), entryKey: "draft",
    command: { type: "text", clientRequestId: "fixture", prompt: "private draft" },
    safeUserText: "private draft", updateEntry, adoptEntry: vi.fn(),
    loadActorState: vi.fn(), refreshConversations: vi.fn(),
  })).rejects.toBeInstanceOf(AccountChangedError);
  expect(updateEntry).toHaveBeenLastCalledWith("draft", { projection: initialProjection, session: base });
  expect(captureNyxbotError).not.toHaveBeenCalled();
});
