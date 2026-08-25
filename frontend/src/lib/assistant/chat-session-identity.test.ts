import { describe, expect, it } from "vitest";

import { AGUIEventType, CustomEventName } from "./agui-types";
import {
  buildConversationSessionSnapshot,
  readConversationPreferences,
  resolveConversationRuntimeIdentity,
} from "./chat-session-identity";

describe("chat session identity", () => {
  it("builds a typed session snapshot from runtime events and conversation preferences", () => {
    const snapshot = buildConversationSessionSnapshot(
      [
        {
          events: [
            {
              runId: "run-1",
              threadId: "thread-1",
              timestamp: 1,
              type: AGUIEventType.RUN_STARTED,
            },
            {
              name: CustomEventName.RunContext,
              timestamp: 2,
              type: AGUIEventType.CUSTOM,
              value: {
                actorId: "actor://nyxid",
                commandId: "cmd-1",
              },
            },
          ],
        },
      ],
      {
        actorId: "",
        commandId: "",
        runId: "",
      },
      {
        llmModel: "gpt-5.4-mini",
        llmRoute: "/api/v1/proxy/s/openai",
      },
    );

    expect(snapshot).toEqual({
      preferences: {
        llmModel: "gpt-5.4-mini",
        llmRoute: "/api/v1/proxy/s/openai",
      },
      runtime: {
        actorId: "actor://nyxid",
        commandId: "cmd-1",
        runId: "run-1",
      },
    });
  });

  it("falls back to stored runtime events when legacy conversation meta does not carry actor identity", () => {
    const identity = resolveConversationRuntimeIdentity({
      meta: {
        createdAt: "2026-04-01T08:00:00.000Z",
        id: "conversation-1",
        messageCount: 2,
        serviceId: "nyxid-chat",
        serviceKind: "nyxid-chat",
        title: "Legacy conversation",
        updatedAt: "2026-04-01T08:05:00.000Z",
      },
      messages: [
        {
          events: [
            {
              runId: "run-restore-1",
              threadId: "thread-restore-1",
              timestamp: 1,
              type: AGUIEventType.RUN_STARTED,
            },
            {
              name: CustomEventName.RunContext,
              timestamp: 2,
              type: AGUIEventType.CUSTOM,
              value: {
                actorId: "actor://restored",
                commandId: "cmd-restore-1",
              },
            },
          ],
        },
      ],
    });

    expect(identity).toEqual({
      actorId: "actor://restored",
      commandId: "cmd-restore-1",
      runId: "run-restore-1",
    });
  });

  it("prefers the structured session preferences when both legacy and typed metadata are present", () => {
    expect(
      readConversationPreferences({
        createdAt: "2026-04-01T08:00:00.000Z",
        id: "conversation-1",
        llmModel: "legacy-model",
        llmRoute: "/legacy",
        messageCount: 1,
        serviceId: "support-service",
        serviceKind: "service",
        session: {
          preferences: {
            llmModel: "gpt-5.4-mini",
            llmRoute: "/api/v1/proxy/s/openai",
          },
        },
        title: "Route override",
        updatedAt: "2026-04-01T08:05:00.000Z",
      }),
    ).toEqual({
      llmModel: "gpt-5.4-mini",
      llmRoute: "/api/v1/proxy/s/openai",
    });
  });
});
