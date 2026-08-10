import { describe, expect, it } from "vitest";
import { resolveAssistantAction } from "@/lib/assistant/action-registry";
import {
  actionContinueBodySchema,
  actionControlIdentitySchema,
  actionWakeBodySchema,
  assistantActionRequestSchema,
  buildActionContinueBody,
  buildActionWakeBody,
} from "./assistant-actions";

const BASE_REQUEST = {
  schemaVersion: 4,
  actorId: "conversation-1",
  originTurnId: "turn-origin-1",
  taskId: "task-1",
  stepId: "step-1",
  actionRequestId: "act-1",
  action: "service.connect",
} as const;

describe("assistant action request schema", () => {
  it("fills protobuf-omitted catalog defaults", () => {
    const request = assistantActionRequestSchema.parse({
      ...BASE_REQUEST,
      params: { catalogService: { serviceSlug: "api-github" } },
    });

    expect(request.params.catalogService).toEqual({
      serviceSlug: "api-github",
      requestedScopes: [],
      viaNodeId: "",
      targetOrgId: "",
    });
    expect(resolveAssistantAction(request)).toMatchObject({
      supported: true,
      journey: "catalog_service",
      params: { variant: "catalog", service_slug: "api-github" },
    });
  });

  it("parses both catalog and custom parameter variants", () => {
    const catalog = assistantActionRequestSchema.parse({
      ...BASE_REQUEST,
      params: {
        catalogService: {
          serviceSlug: "api-github",
          requestedScopes: ["repo"],
          viaNodeId: "node-1",
          targetOrgId: "org-1",
        },
      },
    });
    const custom = assistantActionRequestSchema.parse({
      ...BASE_REQUEST,
      actionRequestId: "act-2",
      params: {
        customService: {
          name: "Build API",
          endpointUrl: "https://build.example.test/v1",
          authMethod: "header",
          authKeyName: "X-Build-Key",
        },
      },
    });

    expect(resolveAssistantAction(catalog).params).toMatchObject({
      variant: "catalog",
      requested_scopes: ["repo"],
      via_node_id: "node-1",
      target_org_id: "org-1",
    });
    expect(resolveAssistantAction(custom)).toMatchObject({
      supported: true,
      journey: "custom_service",
      params: {
        variant: "custom",
        name: "Build API",
        endpoint_url: "https://build.example.test/v1",
        auth_method: "header",
        auth_key_name: "X-Build-Key",
      },
    });
  });

  it("resolves bad catalog slugs and insecure custom urls as unsupported", () => {
    const badSlug = assistantActionRequestSchema.parse({
      ...BASE_REQUEST,
      actionRequestId: "act-bad-slug",
      params: { catalogService: { serviceSlug: "api github" } },
    });
    const httpUrl = assistantActionRequestSchema.parse({
      ...BASE_REQUEST,
      actionRequestId: "act-http",
      params: {
        customService: {
          name: "Build API",
          endpointUrl: "http://build.example.test/v1",
        },
      },
    });
    const queryUrl = assistantActionRequestSchema.parse({
      ...BASE_REQUEST,
      actionRequestId: "act-query",
      params: {
        customService: {
          name: "Build API",
          endpointUrl: "https://build.example.test/v1?token=nope",
        },
      },
    });
    const fragmentUrl = assistantActionRequestSchema.parse({
      ...BASE_REQUEST,
      actionRequestId: "act-fragment",
      params: {
        customService: {
          name: "Build API",
          endpointUrl: "https://build.example.test/v1#secret",
        },
      },
    });
    const badAuthKeyName = assistantActionRequestSchema.parse({
      ...BASE_REQUEST,
      actionRequestId: "act-bad-auth-key",
      params: {
        customService: {
          name: "Build API",
          endpointUrl: "https://build.example.test/v1",
          authMethod: "header",
          authKeyName: "X Auth",
        },
      },
    });

    for (const request of [
      badSlug,
      httpUrl,
      queryUrl,
      fragmentUrl,
      badAuthKeyName,
    ]) {
      expect(resolveAssistantAction(request)).toMatchObject({
        supported: false,
        params: { variant: "unknown" },
      });
    }
  });

  it("routes a wrong schema version and unknown verb to the fallback", () => {
    const wrongVersion = assistantActionRequestSchema.parse({
      ...BASE_REQUEST,
      schemaVersion: 3,
      params: { catalogService: { serviceSlug: "api-github" } },
    });
    const unknownVerb = assistantActionRequestSchema.parse({
      ...BASE_REQUEST,
      actionRequestId: "act-unknown",
      action: "future.action",
      params: {},
    });

    expect(resolveAssistantAction(wrongVersion).supported).toBe(false);
    expect(resolveAssistantAction(unknownVerb).supported).toBe(false);
  });

  it("rejects unknown members and invalid producer-required values", () => {
    const validCatalog = {
      ...BASE_REQUEST,
      params: { catalogService: { serviceSlug: "api-github" } },
    };
    const invalidRequests = [
      { ...validCatalog, unexpected: true },
      { ...validCatalog, actorId: "invalid/actor" },
      {
        ...validCatalog,
        params: {
          catalogService: { serviceSlug: "api-github", unexpected: true },
        },
      },
      {
        ...BASE_REQUEST,
        params: { catalogService: { serviceSlug: "   " } },
      },
      {
        ...BASE_REQUEST,
        action: "   ",
        params: { catalogService: { serviceSlug: "api-github" } },
      },
      {
        ...BASE_REQUEST,
        params: {
          customService: {
            name: "Build API",
            endpointUrl: "https://build.example.test/v1",
            authMethod: "bogus",
          },
        },
      },
      {
        ...BASE_REQUEST,
        params: {
          customService: {
            name: "   ",
            endpointUrl: "https://build.example.test/v1",
            authMethod: "none",
          },
        },
      },
      {
        ...BASE_REQUEST,
        params: {
          catalogService: {
            serviceSlug: "api-github",
            requestedScopes: Array.from({ length: 65 }, (_, index) => {
              return `scope-${String(index)}`;
            }),
          },
        },
      },
    ];

    for (const request of invalidRequests) {
      expect(assistantActionRequestSchema.safeParse(request).success).toBe(
        false,
      );
    }
  });

  it("defaults protobuf-omitted optional fields without opening the schema", () => {
    const request = assistantActionRequestSchema.parse({
      schemaVersion: 4,
      originTurnId: "turn-origin-1",
      actionRequestId: "act-1",
      action: "service.connect",
      params: {
        customService: {
          name: "Build API",
          endpointUrl: "https://build.example.test/v1",
        },
      },
    });

    expect(request).toMatchObject({
      actorId: "",
      taskId: "",
      stepId: "",
      params: {
        customService: {
          authMethod: "none",
          authKeyName: "",
          viaNodeId: "",
          targetOrgId: "",
        },
      },
    });
  });

  it("fails closed on undeclared members and secret-shaped values", () => {
    const undeclaredMember = {
      ...BASE_REQUEST,
      params: {
        customService: {
          name: "Build API",
          endpointUrl: "https://build.example.test/v1",
          authMethod: "header",
          authKeyName: "Authorization",
          deviceCode: "nyx_adc_secret1234",
        },
      },
    };
    const secretValue = {
      ...BASE_REQUEST,
      params: {
        customService: {
          name: "Build API",
          endpointUrl: "https://build.example.test/v1",
          authMethod: "header",
          authKeyName: "X-Auth",
          targetOrgId: "Bearer top-secret-value",
        },
      },
    };

    expect(
      assistantActionRequestSchema.safeParse(undeclaredMember).success,
    ).toBe(false);
    expect(assistantActionRequestSchema.safeParse(secretValue).success).toBe(
      false,
    );
  });
});

describe("action continuation schema", () => {
  it("builds the exact strict completed body with a safe resource ref", () => {
    const body = buildActionContinueBody(
      "nyxid-chat-actor-1",
      "00000000-0000-4000-8000-000000000001",
      "turn-origin-1",
      [
        {
          actionRequestId: "act-1",
          originTurnId: "turn-origin-1",
          disposition: "completed",
          resource: {
            userService: {
              userServiceId: "00000000-0000-4000-8000-000000000002",
            },
          },
        },
      ],
      new Map([["act-1", "service.connect"]]),
    );

    expect(Object.keys(body)).toEqual([
      "type",
      "conversationId",
      "clientRequestId",
      "originTurnId",
      "actions",
    ]);
    expect(Object.keys(body.actions[0] ?? {})).toEqual([
      "actionRequestId",
      "originTurnId",
      "disposition",
      "resource",
    ]);
    expect(body).toEqual({
      type: "action.continue",
      conversationId: "nyxid-chat-actor-1",
      clientRequestId: "00000000-0000-4000-8000-000000000001",
      originTurnId: "turn-origin-1",
      actions: [
        {
          actionRequestId: "act-1",
          originTurnId: "turn-origin-1",
          disposition: "completed",
          resource: {
            userService: {
              userServiceId: "00000000-0000-4000-8000-000000000002",
            },
          },
        },
      ],
    });
  });

  it("rejects empty actions, mixed origins, duplicates, and extra members", () => {
    const validReport = {
      actionRequestId: "act-1",
      originTurnId: "turn-origin-1",
      disposition: "declined",
    } as const;
    const base = {
      type: "action.continue",
      conversationId: "nyxid-chat-actor-1",
      clientRequestId: "request-1",
      originTurnId: "turn-origin-1",
    } as const;

    expect(
      actionContinueBodySchema.safeParse({ ...base, actions: [] }).success,
    ).toBe(false);
    expect(
      actionContinueBodySchema.safeParse({
        ...base,
        actions: [{ ...validReport, originTurnId: "turn-other" }],
      }).success,
    ).toBe(false);
    expect(
      actionContinueBodySchema.safeParse({
        ...base,
        actions: [validReport, validReport],
      }).success,
    ).toBe(false);
    expect(
      actionContinueBodySchema.safeParse({
        ...base,
        prompt: "must not be sent",
        actions: [validReport],
      }).success,
    ).toBe(false);
    expect(
      actionContinueBodySchema.safeParse({
        ...base,
        actions: [
          {
            ...validReport,
            resource: {
              userService: { userServiceId: "service-1" },
              key: { keyId: "key-1" },
            },
          },
        ],
      }).success,
    ).toBe(false);
  });

  it("builds an exact empty out-of-band wake without accepting reports", () => {
    const body = buildActionWakeBody(
      "nyxid-chat-actor-1",
      "request-wake-1",
      "turn-origin-1",
    );

    expect(body).toEqual({
      type: "action.continue",
      conversationId: "nyxid-chat-actor-1",
      clientRequestId: "request-wake-1",
      originTurnId: "turn-origin-1",
      actions: [],
    });
    expect(
      actionWakeBodySchema.safeParse({
        ...body,
        actions: [
          {
            actionRequestId: "act-1",
            originTurnId: "turn-origin-1",
            disposition: "completed",
          },
        ],
      }).success,
    ).toBe(false);
    expect(
      actionWakeBodySchema.safeParse({ ...body, prompt: "must not be sent" })
        .success,
    ).toBe(false);
  });

  it("requires the resource variant owned by service and key actions", () => {
    expect(() =>
      buildActionContinueBody(
        "nyxid-chat-actor-1",
        "request-1",
        "turn-origin-1",
        [
          {
            actionRequestId: "act-1",
            originTurnId: "turn-origin-1",
            disposition: "completed",
          },
        ],
        new Map([["act-1", "service.connect"]]),
      ),
    ).toThrow("Completed action reports must include a resource reference");
    expect(() =>
      buildActionContinueBody(
        "nyxid-chat-actor-1",
        "request-1",
        "turn-origin-1",
        [
          {
            actionRequestId: "act-1",
            originTurnId: "turn-origin-1",
            disposition: "completed",
            resource: { key: { keyId: "key-1" } },
          },
        ],
        new Map([["act-1", "service.reauthorize"]]),
      ),
    ).toThrow(
      "service.reauthorize completed reports must include resource.userService.userServiceId",
    );
    expect(() =>
      buildActionContinueBody(
        "nyxid-chat-actor-1",
        "request-1",
        "turn-origin-1",
        [
          {
            actionRequestId: "act-1",
            originTurnId: "turn-origin-1",
            disposition: "completed",
            resource: {
              userService: { userServiceId: "service-1" },
            },
          },
        ],
        new Map([["act-1", "key.rotate"]]),
      ),
    ).toThrow("key.rotate completed reports must include resource.key.keyId");
  });

  it("round-trips all six safe resource variants when the action is neutral", () => {
    const resources = [
      { userService: { userServiceId: "service-1" } },
      { key: { keyId: "key-1" } },
      { node: { nodeId: "node-1" } },
      { serviceAccount: { serviceAccountId: "sa-1" } },
      { developerApp: { clientId: "app-1" } },
      { device: { deviceId: "device-1" } },
    ] as const;

    for (const [index, resource] of resources.entries()) {
      const actionRequestId = `act-${String(index)}`;
      const body = buildActionContinueBody(
        "nyxid-chat-actor-1",
        `request-${String(index)}`,
        "turn-origin-1",
        [
          {
            actionRequestId,
            originTurnId: "turn-origin-1",
            disposition: "completed",
            resource,
          },
        ],
        new Map([[actionRequestId, "node.inspect"]]),
      );
      expect(body.actions[0]?.resource).toEqual(resource);
    }
  });

  it("fails closed for completed reports without resources", () => {
    expect(() =>
      buildActionContinueBody(
        "nyxid-chat-actor-1",
        "request-1",
        "turn-origin-1",
        [
          {
            actionRequestId: "act-1",
            originTurnId: "turn-origin-1",
            disposition: "completed",
          },
        ],
        new Map(),
      ),
    ).toThrow("Completed action reports must include a resource reference");
  });

  it("enforces the control-identity character and length rules", () => {
    for (const invalid of [
      "has space",
      "has/slash",
      "has\\backslash",
      "has?query",
      "has#fragment",
      "line\nbreak",
      "x".repeat(257),
    ]) {
      expect(actionControlIdentitySchema.safeParse(invalid).success).toBe(
        false,
      );
    }
    expect(
      actionControlIdentitySchema.safeParse("turn-safe_1:part").success,
    ).toBe(true);
  });
});
