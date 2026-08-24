import { describe, expect, it } from "vitest";
import { resolveAssistantAction } from "@/lib/assistant/action-registry";
import {
  actionContinueBodySchema,
  actionControlIdentitySchema,
  actionResourceSchema,
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

const WAVE_3_4_ACTIONS = [
  {
    action: "node.register_token",
    params: { name: "Edge node", targetOrgId: "org-1" },
    variant: "node_register_token",
  },
  {
    action: "node.rotate_token",
    params: { nodeId: "node-1" },
    variant: "node_rotate_token",
  },
  {
    action: "node.delete",
    params: { nodeId: "node-1" },
    variant: "node_delete",
  },
  {
    action: "node.transfer",
    params: { nodeId: "node-1", newOwnerUserId: "user-2" },
    variant: "node_transfer",
  },
  {
    action: "node.inject_credential",
    params: {
      nodeId: "node-1",
      serviceSlug: "github",
      injectionMethod: "header",
      fieldName: "Authorization",
      targetUrl: "https://api.github.test",
      label: "GitHub",
    },
    variant: "node_inject_credential",
  },
  {
    action: "pending_credential.push",
    params: {
      nodeId: "node-1",
      serviceSlug: "github",
      injectionMethod: "query-param",
      fieldName: "token",
    },
    variant: "pending_credential_push",
  },
  {
    action: "pending_credential.cancel",
    params: { nodeId: "node-1", pendingCredentialId: "pending-1" },
    variant: "pending_credential_cancel",
  },
  {
    action: "device.onboard",
    params: {
      label: "Kitchen",
      targetOrgId: "org-1",
      defaultServiceIds: ["service-1"],
    },
    variant: "device_onboard",
  },
  {
    action: "org.create",
    params: {
      displayName: "Platform",
      contactEmail: "platform@example.test",
      avatarUrl: "https://example.test/avatar.png",
    },
    variant: "org_create",
  },
  {
    action: "org.update",
    params: {
      orgId: "org-1",
      displayName: "Platform Ops",
      slug: "platform-ops",
    },
    variant: "org_update",
  },
  { action: "org.delete", params: { orgId: "org-1" }, variant: "org_delete" },
  {
    action: "org.member_add",
    params: {
      orgId: "org-1",
      userId: "user-1",
      role: "member",
      allowedServiceIds: ["service-1"],
    },
    variant: "org_member_add",
  },
  {
    action: "org.member_remove",
    params: { orgId: "org-1", memberId: "member-1" },
    variant: "org_member_remove",
  },
  {
    action: "org.member_update_role",
    params: { orgId: "org-1", memberId: "member-1", role: "admin" },
    variant: "org_member_update_role",
  },
  {
    action: "org.invite",
    params: {
      orgId: "org-1",
      role: "viewer",
      allowedServiceIds: ["service-1"],
    },
    variant: "org_invite",
  },
  {
    action: "org.set_primary",
    params: { orgId: "org-1" },
    variant: "org_set_primary",
  },
  {
    action: "account.profile_update",
    params: { displayName: "Ada", avatarUrl: "https://example.test/ada.png" },
    variant: "account_profile_update",
  },
  {
    action: "account.revoke_consent",
    params: { clientId: "client-1" },
    variant: "account_revoke_consent",
  },
  { action: "account.delete", params: {}, variant: "account_delete" },
  { action: "account.mfa_setup", params: {}, variant: "account_mfa_setup" },
  {
    action: "approval.configure",
    params: { serviceId: "service-1" },
    variant: "approval_configure",
  },
  {
    action: "approval.enable",
    params: { serviceId: "service-1" },
    variant: "approval_enable",
  },
  {
    action: "approval.disable",
    params: { serviceId: "service-1" },
    variant: "approval_disable",
  },
  {
    action: "approval.revoke_grant",
    params: { grantId: "grant-1" },
    variant: "approval_revoke_grant",
  },
  {
    action: "notifications.update",
    params: {},
    variant: "notifications_update",
  },
  {
    action: "notifications.telegram_link",
    params: {},
    variant: "notifications_telegram_link",
  },
  {
    action: "notifications.telegram_disconnect",
    params: {},
    variant: "notifications_telegram_disconnect",
  },
  {
    action: "service_account.create",
    params: { name: "Deploy agent", description: "Production deploys" },
    variant: "service_account_create",
  },
  {
    action: "service_account.update",
    params: { serviceAccountId: "service-account-1", name: "Deploy agent v2" },
    variant: "service_account_update",
  },
  {
    action: "service_account.delete",
    params: { serviceAccountId: "service-account-1" },
    variant: "service_account_delete",
  },
  {
    action: "service_account.rotate_secret",
    params: { serviceAccountId: "service-account-1" },
    variant: "service_account_rotate_secret",
  },
  {
    action: "service_account.revoke_tokens",
    params: { serviceAccountId: "service-account-1" },
    variant: "service_account_revoke_tokens",
  },
  {
    action: "developer_app.create",
    params: {
      name: "Console",
      redirectUris: ["https://console.example.test/callback"],
    },
    variant: "developer_app_create",
  },
  {
    action: "developer_app.update",
    params: {
      clientId: "client-1",
      name: "Console v2",
      redirectUris: ["https://console.example.test/oauth/callback"],
    },
    variant: "developer_app_update",
  },
  {
    action: "developer_app.delete",
    params: { clientId: "client-1" },
    variant: "developer_app_delete",
  },
  {
    action: "developer_app.rotate_secret",
    params: { clientId: "client-1" },
    variant: "developer_app_rotate_secret",
  },
  {
    action: "external_key.add_gcp_service_account",
    params: { label: "GCP production" },
    variant: "external_key_add_gcp_service_account",
  },
  {
    action: "openclaw.connect",
    params: { gatewayUrl: "https://openclaw.example.test" },
    variant: "openclaw_connect",
  },
] as const;

describe("assistant action request schema", () => {
  it("fills protobuf-omitted catalog defaults", () => {
    const request = assistantActionRequestSchema.parse({
      ...BASE_REQUEST,
      params: { catalogService: { serviceSlug: "api-github" } },
    });

    expect(request.params).toMatchObject({
      catalogService: {
        serviceSlug: "api-github",
        requestedScopes: [],
        viaNodeId: "",
        targetOrgId: "",
      },
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

  it("parses exact service.reauthorize params including URL-shaped scopes", () => {
    const request = assistantActionRequestSchema.parse({
      ...BASE_REQUEST,
      action: "service.reauthorize",
      params: {
        userServiceId: "service-alpha",
        requestedScopes: [
          "repo",
          "https://www.googleapis.com/auth/cloud-platform.read-only",
        ],
      },
    });

    expect(resolveAssistantAction(request)).toMatchObject({
      supported: true,
      journey: "service_reauthorize",
      params: {
        variant: "service_reauthorize",
        user_service_id: "service-alpha",
        requested_scopes: [
          "repo",
          "https://www.googleapis.com/auth/cloud-platform.read-only",
        ],
      },
    });
  });

  it("rejects empty, duplicate, unnormalized, packed, and widened reauthorization scopes", () => {
    const invalidParams = [
      { userServiceId: "service-alpha", requestedScopes: [] },
      {
        userServiceId: "service-alpha",
        requestedScopes: ["repo", "repo"],
      },
      { userServiceId: "service-alpha", requestedScopes: [" repo"] },
      { userServiceId: "service-alpha", requestedScopes: ["repo write"] },
      { userServiceId: "service-alpha", requestedScopes: ["repo,write"] },
      {
        userServiceId: "service-alpha",
        requestedScopes: ["repo"],
        replacementServiceId: "service-beta",
      },
      { userServiceId: "invalid/service", requestedScopes: ["repo"] },
    ];

    for (const [index, params] of invalidParams.entries()) {
      expect(
        assistantActionRequestSchema.safeParse({
          ...BASE_REQUEST,
          actionRequestId: `invalid-reauthorize-${String(index)}`,
          action: "service.reauthorize",
          params,
        }).success,
      ).toBe(false);
    }
  });

  it("falls back to unsupported when service.reauthorize params are absent", () => {
    const request = assistantActionRequestSchema.parse({
      ...BASE_REQUEST,
      action: "service.reauthorize",
      params: {},
    });

    expect(resolveAssistantAction(request)).toMatchObject({
      supported: false,
      journey: null,
      params: { variant: "unknown" },
    });
  });

  it("parses an exact nonempty key.create service set", () => {
    const request = assistantActionRequestSchema.parse({
      ...BASE_REQUEST,
      action: "key.create",
      params: {
        name: " coding-agent ",
        platform: " codex ",
        allowedServiceIds: ["service-alpha", "service-beta"],
      },
    });

    expect(resolveAssistantAction(request)).toMatchObject({
      supported: true,
      journey: "key_create",
      params: {
        variant: "key_create",
        name: "coding-agent",
        platform: "codex",
        allowed_service_ids: ["service-alpha", "service-beta"],
      },
    });
  });

  it("parses only an exact key.rotate predecessor identity", () => {
    const request = assistantActionRequestSchema.parse({
      ...BASE_REQUEST,
      action: "key.rotate",
      params: { keyId: "key-predecessor-alpha" },
    });

    expect(resolveAssistantAction(request)).toMatchObject({
      supported: true,
      journey: "key_rotate",
      params: {
        variant: "key_rotate",
        key_id: "key-predecessor-alpha",
      },
    });

    const missing = assistantActionRequestSchema.parse({
      ...BASE_REQUEST,
      actionRequestId: "missing-rotate-params",
      action: "key.rotate",
      params: {},
    });
    expect(resolveAssistantAction(missing).supported).toBe(false);

    for (const params of [
      { keyId: "" },
      { keyId: "invalid/key" },
      { keyId: "key-alpha", successorId: "key-beta" },
      { keyId: "key-alpha", fullKey: "nyxid_ag_forbidden" },
    ]) {
      expect(
        assistantActionRequestSchema.safeParse({
          ...BASE_REQUEST,
          actionRequestId: `invalid-rotate-${JSON.stringify(params)}`,
          action: "key.rotate",
          params,
        }).success,
      ).toBe(false);
    }
  });

  it("accepts all twelve Wave-2 param shapes and resolves their journeys", () => {
    const wave2 = [
      ["key.update", { keyId: "key-1", name: "renamed" }],
      ["key.delete", { keyId: "key-1" }],
      [
        "key.extend_scope",
        { keyId: "key-1", addServiceIds: ["service-alpha"] },
      ],
      [
        "key.bind_credential",
        {
          keyId: "key-1",
          userServiceId: "service-alpha",
          externalKeyId: "external-1",
        },
      ],
      [
        "service.update",
        { userServiceId: "service-alpha", name: "Renamed API" },
      ],
      ["service.delete", { userServiceId: "service-alpha" }],
      [
        "service.route",
        { userServiceId: "service-alpha", viaNodeId: "node-1" },
      ],
      ["service.rotate_credential", { userServiceId: "service-alpha" }],
      ["endpoint.update", { endpointId: "endpoint-1", label: "Renamed" }],
      ["endpoint.delete", { endpointId: "endpoint-1" }],
      ["external_key.rotate", { externalKeyId: "external-1" }],
      ["external_key.delete", { externalKeyId: "external-1" }],
    ] as const;

    for (const [index, [action, params]] of wave2.entries()) {
      const request = assistantActionRequestSchema.parse({
        ...BASE_REQUEST,
        actionRequestId: `wave2-${String(index)}`,
        action,
        params,
      });
      // Wave 2's journeys have landed, so these now resolve to a real
      // variant. Dormancy still holds where it matters: these verbs are
      // absent from every Aevatar pinned revision set, so no card for them
      // can arrive until the v9 bump. The browser being ready is the point.
      //
      // Falsifier: drop a Wave-2 registry row and its entry here resolves to
      // `unknown` again, failing this assertion.
      const resolved = resolveAssistantAction(request);
      expect(resolved.supported).toBe(true);
      expect(resolved.params.variant).not.toBe("unknown");
    }
  });

  it.each(WAVE_3_4_ACTIONS)(
    "parses and resolves $action as $variant",
    ({ action, params, variant }) => {
      const request = assistantActionRequestSchema.parse({
        ...BASE_REQUEST,
        actionRequestId: `active-${variant}`,
        action,
        params,
      });
      expect(resolveAssistantAction(request)).toMatchObject({
        supported: true,
        journey: variant,
        params: { variant },
      });
    },
  );

  it("rejects or leaves unsupported id-less, widened, and secret-carrying Wave-2 params", () => {
    const invalid = [
      ["key.update", { name: "renamed" }],
      ["key.extend_scope", { keyId: "key-1", addServiceIds: [] }],
      [
        "key.extend_scope",
        { keyId: "key-1", addServiceIds: ["service-alpha", "service-alpha"] },
      ],
      [
        "key.bind_credential",
        { keyId: "key-1", userServiceId: "service-alpha" },
      ],
      [
        "service.update",
        { userServiceId: "service-alpha", authMethod: "bogus" },
      ],
      [
        "service.route",
        { userServiceId: "service-alpha", viaNodeId: "invalid/node" },
      ],
      [
        "service.rotate_credential",
        {
          userServiceId: "service-alpha",
          credentialValue: "nyxid_ag_secret99",
        },
      ],
      ["endpoint.update", { label: "Renamed" }],
      [
        "external_key.rotate",
        { externalKeyId: "external-1", replacement: "Bearer top-secret" },
      ],
    ] as const;

    for (const [index, [action, params]] of invalid.entries()) {
      const parsed = assistantActionRequestSchema.safeParse({
        ...BASE_REQUEST,
        actionRequestId: `wave2-invalid-${String(index)}`,
        action,
        params,
      });
      if (parsed.success) {
        expect(resolveAssistantAction(parsed.data).supported).toBe(false);
      }
    }
  });

  it("rejects missing, empty, duplicate, malformed, and widened key.create params", () => {
    const invalidParams = [
      { name: "agent", platform: "codex" },
      { name: "agent", platform: "codex", allowedServiceIds: [] },
      {
        name: "agent",
        platform: "codex",
        allowedServiceIds: ["service-alpha", "service-alpha"],
      },
      {
        name: "agent",
        platform: "codex",
        allowedServiceIds: ["invalid/service"],
      },
      {
        name: "agent",
        platform: "codex",
        allowedServiceIds: ["service-alpha"],
        allowAllServices: true,
      },
    ];

    for (const [index, params] of invalidParams.entries()) {
      expect(
        assistantActionRequestSchema.safeParse({
          ...BASE_REQUEST,
          actionRequestId: `invalid-key-create-${String(index)}`,
          action: "key.create",
          params,
        }).success,
      ).toBe(false);
    }
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
        new Map([["act-1", "key.bind_credential"]]),
      ),
    ).toThrow(
      "key.bind_credential completed reports must include resource.key.userServiceId",
    );
    const bindBody = buildActionContinueBody(
      "nyxid-chat-actor-1",
      "request-1",
      "turn-origin-1",
      [
        {
          actionRequestId: "act-1",
          originTurnId: "turn-origin-1",
          disposition: "completed",
          resource: {
            key: { keyId: "key-1", userServiceId: "svc-1" },
          },
        },
      ],
      new Map([["act-1", "key.bind_credential"]]),
    );
    expect(bindBody.actions[0]?.resource).toEqual({
      key: { keyId: "key-1", userServiceId: "svc-1" },
    });
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

describe("wave-2 typed resource variants", () => {
  it("accepts the endpoint and externalKey variants", () => {
    expect(
      actionResourceSchema.safeParse({ endpoint: { endpointId: "ep-1" } })
        .success,
    ).toBe(true);
    expect(
      actionResourceSchema.safeParse({
        externalKey: { externalKeyId: "xk-1" },
      }).success,
    ).toBe(true);
  });

  it("rejects unknown members inside the new variants", () => {
    expect(
      actionResourceSchema.safeParse({
        endpoint: { endpointId: "ep-1", label: "leak" },
      }).success,
    ).toBe(false);
    expect(
      actionResourceSchema.safeParse({
        externalKey: { externalKeyId: "xk-1", credential: "nyxid_abcdefgh" },
      }).success,
    ).toBe(false);
  });

  // api_keys and user_api_keys are different collections: a `key` variant is
  // resolved against /api/v1/api-keys/{id}/authorization, so an external-key
  // id sent that way reads the wrong collection entirely.
  it("keeps externalKey distinct from key", () => {
    const asKey = actionResourceSchema.safeParse({
      key: { keyId: "xk-1" },
    });
    const asExternal = actionResourceSchema.safeParse({
      externalKey: { externalKeyId: "xk-1" },
    });
    expect(asKey.success && asExternal.success).toBe(true);
    expect(JSON.stringify(asKey.data)).not.toEqual(
      JSON.stringify(asExternal.data),
    );
  });
});
