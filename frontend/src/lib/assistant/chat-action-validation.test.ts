import { describe, expect, it } from "vitest";

import { resolveAssistantAction } from "@/lib/assistant/action-registry";
import { validateActionRequest } from "./chat-action-validation";

const baseRequest = {
  schemaVersion: 4,
  actorId: "conversation-alpha",
  originTurnId: "turn-alpha",
  taskId: "task-alpha",
  stepId: "step-alpha",
  actionRequestId: "action-alpha",
} as const;

describe("chat action validation", () => {
  it("accepts a supported non-service.connect v4 action", () => {
    const request = validateActionRequest({
      ...baseRequest,
      action: "key.create",
      params: {
        name: "Automation key",
        platform: "codex",
        allowedServiceIds: ["service-alpha"],
      },
    });

    expect(request.action).toBe("key.create");
    expect(resolveAssistantAction(request).supported).toBe(true);
  });

  it("preserves structurally valid future and deferred actions", () => {
    const future = validateActionRequest({
      ...baseRequest,
      schemaVersion: 5,
      action: "future.action",
      params: {},
    });
    const deferred = validateActionRequest({
      ...baseRequest,
      action: "node.delete",
      params: {},
    });

    expect(resolveAssistantAction(future).supported).toBe(false);
    expect(resolveAssistantAction(deferred).supported).toBe(false);
  });

  it("rejects undeclared fields at every strict schema boundary", () => {
    expect(() =>
      validateActionRequest({
        ...baseRequest,
        action: "node.delete",
        extra: 1,
      }),
    ).toThrow(expect.objectContaining({ code: "NYXID_FIELD_UNDECLARED" }));
    expect(() =>
      validateActionRequest({
        ...baseRequest,
        action: "key.create",
        params: {
          name: "Automation key",
          platform: "codex",
          allowedServiceIds: ["service-alpha"],
          extra: true,
        },
      }),
    ).toThrow(expect.objectContaining({ code: "NYXID_FIELD_UNDECLARED" }));
  });

  it("rejects secret-shaped values in otherwise declared fields", () => {
    expect(() =>
      validateActionRequest({
        ...baseRequest,
        action: "key.create",
        params: {
          name: "Bearer abcdefghijklmnop",
          platform: "codex",
          allowedServiceIds: ["service-alpha"],
        },
      }),
    ).toThrow(expect.objectContaining({ code: "NYXID_SECRET_FORBIDDEN" }));
  });

  it("requires every actor action identity", () => {
    expect(() =>
      validateActionRequest({
        ...baseRequest,
        actorId: "conversation/alpha",
        action: "node.delete",
        params: {},
      }),
    ).toThrow(expect.objectContaining({ code: "NYXID_IDENTITY_INVALID" }));
  });

  it("rejects unsafe custom-service endpoints before they reach a journey", () => {
    expect(() =>
      validateActionRequest({
        ...baseRequest,
        action: "service.connect",
        params: {
          customService: {
            name: "Unsafe service",
            endpointUrl: "http://example.com/api?token=visible",
            authMethod: "none",
          },
        },
      }),
    ).toThrow(expect.objectContaining({ code: "NYXID_URL_UNSAFE" }));
  });
});
