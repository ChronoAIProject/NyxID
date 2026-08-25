import { describe, expect, it } from "vitest";

import { validateActionRequest } from "./chat-action-validation";

const baseRequest = {
  schemaVersion: 4,
  actorId: "conversation-alpha",
  originTurnId: "turn-alpha",
  taskId: "task-alpha",
  stepId: "step-alpha",
  actionRequestId: "action-alpha",
} as const;

const supportedRequest = {
  ...baseRequest,
  action: "key.create",
  params: {
    name: "Automation key",
    platform: "codex",
    allowedServiceIds: ["service-alpha"],
  },
} as const;

function expectRecovered(input: unknown) {
  const result = validateActionRequest(input);
  expect(result).toMatchObject({ supported: false, recovered: true });
  expect(result.request).toMatchObject({
    actionRequestId: "action-alpha",
    originTurnId: "turn-alpha",
    params: {},
  });
  return result;
}

describe("chat action validation", () => {
  it("accepts a supported non-service.connect v4 action", () => {
    expect(validateActionRequest(supportedRequest)).toMatchObject({
      request: supportedRequest,
      supported: true,
      recovered: false,
    });
  });

  it("recovers malformed supported-action params as unsupported", () => {
    expectRecovered({ ...baseRequest, action: "key.create", params: {} });
  });

  it("recovers undeclared fields as unsupported", () => {
    const recovered = expectRecovered({
      ...supportedRequest,
      extra: "future-field",
    });
    expect(recovered).toMatchObject({ reason: "undeclared_field" });
  });

  it("recovers unknown actions as unsupported", () => {
    expectRecovered({ ...baseRequest, action: "future.action", params: {} });
  });

  it("recovers future schema versions as unsupported", () => {
    expectRecovered({ ...supportedRequest, schemaVersion: 5 });
  });

  it("recovers deferred registry wiring as unsupported", () => {
    expectRecovered({ ...baseRequest, action: "node.delete", params: {} });
  });

  it("recovers unsafe custom-service endpoints as unsupported", () => {
    expectRecovered({
      ...baseRequest,
      action: "service.connect",
      params: {
        customService: {
          name: "Unsafe service",
          endpointUrl: "http://example.com/api?visible=true",
          authMethod: "none",
        },
      },
    });
  });

  it("rejects secret-bearing params and identities", () => {
    expect(() =>
      validateActionRequest({
        ...supportedRequest,
        params: { ...supportedRequest.params, name: "Bearer abcdefghijklmnop" },
      }),
    ).toThrow(expect.objectContaining({ code: "NYXID_SECRET_FORBIDDEN" }));
    expect(() =>
      validateActionRequest({
        ...supportedRequest,
        actorId: "nyxid_secretidentity",
      }),
    ).toThrow(expect.objectContaining({ code: "NYXID_SECRET_FORBIDDEN" }));
  });

  it("rejects payloads without recoverable control identities", () => {
    expect(() => validateActionRequest(null)).toThrow(
      expect.objectContaining({ code: "NYXID_ACTION_VARIANT_INVALID" }),
    );
    expect(() =>
      validateActionRequest({
        ...supportedRequest,
        originTurnId: undefined,
        actionRequestId: undefined,
      }),
    ).toThrow(
      expect.objectContaining({ code: "NYXID_ACTION_VARIANT_INVALID" }),
    );
  });
});
