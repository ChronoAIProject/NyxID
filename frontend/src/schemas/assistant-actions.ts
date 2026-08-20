import { z } from "zod";

export const ACTION_SCHEMA_VERSION = 4;
export const ACTION_SERVICE_SLUG_PATTERN = /^[A-Za-z0-9._-]{1,128}$/;

// Aevatar uses these values as control-plane identities. Keep this stricter
// than a generic non-empty string so an invalid report never reaches /stream.
export const actionControlIdentitySchema = z
  .string()
  .min(1)
  .max(256)
  // eslint-disable-next-line no-control-regex -- mirrors the live Aevatar DTO guard
  .regex(/^[^\s\x00-\x1f\x7f/\\?#]+$/, "Invalid control identity");

const wireStringSchema = z.string().max(4_096).optional().default("");
const wireIdSchema = z.string().max(256).optional().default("");
const optionalControlIdentitySchema = z
  .union([z.literal(""), actionControlIdentitySchema])
  .optional()
  .default("");
const requiredWireStringSchema = z
  .string()
  .max(4_096)
  .transform((value) => value.trim())
  .pipe(z.string().min(1));
const requiredActionIdentitySchema = z
  .string()
  .transform((value) => value.trim())
  .pipe(actionControlIdentitySchema);

const allowedServiceIdsSchema = z
  .array(requiredActionIdentitySchema)
  .min(1)
  .max(64)
  .superRefine((values, context) => {
    const seen = new Set<string>();
    for (const [index, value] of values.entries()) {
      if (seen.has(value)) {
        context.addIssue({
          code: "custom",
          path: [index],
          message: "Duplicate allowed service id",
        });
      }
      seen.add(value);
    }
  });

export const actionScopeTokenSchema = z
  .string()
  .min(1)
  .max(256)
  .refine((value) => value === value.trim(), "Scope must be normalized")
  .regex(
    /^[A-Za-z0-9._:/~+*=-]+$/,
    "Scope contains characters NyxID cannot request",
  );

const requestedScopesSchema = z
  .array(actionScopeTokenSchema)
  .min(1)
  .max(64)
  .superRefine((values, context) => {
    const seen = new Set<string>();
    for (const [index, value] of values.entries()) {
      if (seen.has(value)) {
        context.addIssue({
          code: "custom",
          path: [index],
          message: "Duplicate requested scope",
        });
      }
      seen.add(value);
    }
  });

const SECRET_VALUE =
  /(Bearer\s+)[A-Za-z0-9._~+/-]+|nyx(?:id)?_[A-Za-z0-9_-]{8,}/gi;
const FORBIDDEN_ACTION_KEY =
  /(?:^|[_-])(authorization|api[-_]?key|token|secret|password|credential|cookie|user[-_]?code|device[-_]?code)(?:$|[_-])/i;

export const customServiceAuthMethodSchema = z.enum([
  "bearer",
  "header",
  "query",
  "path",
  "basic",
  "body",
  "none",
]);

export const catalogServiceActionParamsSchema = z
  .object({
    serviceSlug: requiredWireStringSchema,
    requestedScopes: z
      .array(z.string().max(256))
      .max(64)
      .optional()
      .default([]),
    viaNodeId: wireIdSchema,
    targetOrgId: wireIdSchema,
  })
  .strict();

export const customServiceActionParamsSchema = z
  .object({
    name: requiredWireStringSchema,
    endpointUrl: requiredWireStringSchema,
    authMethod: customServiceAuthMethodSchema.optional().default("none"),
    authKeyName: wireStringSchema,
    viaNodeId: wireIdSchema,
    targetOrgId: wireIdSchema,
  })
  .strict();

export const serviceConnectActionParamsSchema = z
  .object({
    catalogService: catalogServiceActionParamsSchema.optional(),
    customService: customServiceActionParamsSchema.optional(),
  })
  .strict();

export const serviceReauthorizeActionParamsSchema = z
  .object({
    userServiceId: requiredActionIdentitySchema,
    requestedScopes: requestedScopesSchema,
  })
  .strict();

export const keyCreateActionParamsSchema = z
  .object({
    name: z
      .string()
      .max(200)
      .transform((value) => value.trim())
      .pipe(z.string().min(1)),
    platform: z
      .string()
      .max(100)
      .transform((value) => value.trim())
      .pipe(z.string().min(1)),
    allowedServiceIds: allowedServiceIdsSchema,
  })
  .strict();

export const keyRotateActionParamsSchema = z
  .object({
    keyId: requiredActionIdentitySchema,
  })
  .strict();

// ---- Wave 2 param shapes (WS-0; dormant until an Aevatar revision pins the
// verbs). Journey resolution stays "unsupported" in action-registry until the
// owning team lands its journey, so accepting the shapes here changes no
// behavior. Optional members deliberately have NO defaults: protobuf elides
// empty strings, and for update-shaped verbs absence means "leave unchanged".
const optionalUpdateTextSchema = z.string().max(4_096).optional();

export const keyUpdateActionParamsSchema = z
  .object({
    keyId: requiredActionIdentitySchema,
    name: optionalUpdateTextSchema,
    platform: optionalUpdateTextSchema,
    description: optionalUpdateTextSchema,
  })
  .strict();

export const keyDeleteActionParamsSchema = z
  .object({
    keyId: requiredActionIdentitySchema,
  })
  .strict();

export const keyExtendScopeActionParamsSchema = z
  .object({
    keyId: requiredActionIdentitySchema,
    addServiceIds: allowedServiceIdsSchema,
  })
  .strict();

export const keyBindCredentialActionParamsSchema = z
  .object({
    keyId: requiredActionIdentitySchema,
    userServiceId: requiredActionIdentitySchema,
    externalKeyId: requiredActionIdentitySchema,
  })
  .strict();

export const serviceUpdateActionParamsSchema = z
  .object({
    userServiceId: requiredActionIdentitySchema,
    name: optionalUpdateTextSchema,
    endpointUrl: optionalUpdateTextSchema,
    authMethod: customServiceAuthMethodSchema.optional(),
    authKeyName: optionalUpdateTextSchema,
  })
  .strict();

export const serviceDeleteActionParamsSchema = z
  .object({
    userServiceId: requiredActionIdentitySchema,
  })
  .strict();

export const serviceRouteActionParamsSchema = z
  .object({
    userServiceId: requiredActionIdentitySchema,
    viaNodeId: actionControlIdentitySchema.optional(),
  })
  .strict();

export const serviceRotateCredentialActionParamsSchema = z
  .object({
    userServiceId: requiredActionIdentitySchema,
  })
  .strict();

export const endpointUpdateActionParamsSchema = z
  .object({
    endpointId: requiredActionIdentitySchema,
    label: optionalUpdateTextSchema,
    endpointUrl: optionalUpdateTextSchema,
    openapiSpecUrl: optionalUpdateTextSchema,
  })
  .strict();

export const endpointDeleteActionParamsSchema = z
  .object({
    endpointId: requiredActionIdentitySchema,
  })
  .strict();

export const externalKeyRotateActionParamsSchema = z
  .object({
    externalKeyId: requiredActionIdentitySchema,
  })
  .strict();

export const externalKeyDeleteActionParamsSchema = z
  .object({
    externalKeyId: requiredActionIdentitySchema,
  })
  .strict();

const emptyActionParamsSchema = z.object({}).strict();

export const assistantActionParamsSchema = z
  .union([
    serviceConnectActionParamsSchema,
    serviceReauthorizeActionParamsSchema,
    keyCreateActionParamsSchema,
    keyRotateActionParamsSchema,
    keyUpdateActionParamsSchema,
    keyExtendScopeActionParamsSchema,
    keyBindCredentialActionParamsSchema,
    serviceUpdateActionParamsSchema,
    serviceRouteActionParamsSchema,
    endpointUpdateActionParamsSchema,
    endpointDeleteActionParamsSchema,
    externalKeyRotateActionParamsSchema,
    emptyActionParamsSchema,
  ])
  .optional()
  .default({});

/**
 * Structural schema for the CUSTOM payload. Schema version and verb are
 * intentionally not literals: structurally valid future requests must render
 * the unsupported fallback card so the user can explicitly decline them.
 */
export const assistantActionRequestSchema = z
  .object({
    schemaVersion: z.number().int().optional().default(0),
    actorId: optionalControlIdentitySchema,
    originTurnId: actionControlIdentitySchema,
    taskId: wireIdSchema,
    stepId: wireIdSchema,
    actionRequestId: actionControlIdentitySchema,
    action: requiredWireStringSchema,
    params: assistantActionParamsSchema,
  })
  .strict()
  .superRefine((request, context) => {
    const secretPath = findSecretPath(request);
    if (!secretPath) return;
    context.addIssue({
      code: "custom",
      path: [...secretPath],
      message: "Action request contained secret material",
    });
  });

export type AssistantActionRequest = z.infer<
  typeof assistantActionRequestSchema
>;

/**
 * Preserve the user's escape hatch when a recognizable action frame has
 * malformed params. Only the two control identities needed by
 * `action.continue` survive; every failed field and unknown member is dropped.
 */
export function recoverUnsupportedAssistantActionRequest(
  payload: unknown,
): AssistantActionRequest | null {
  if (!payload || typeof payload !== "object" || Array.isArray(payload)) {
    return null;
  }
  const record = payload as Record<string, unknown>;
  const actionRequestId = actionControlIdentitySchema.safeParse(
    record["actionRequestId"],
  );
  const originTurnId = actionControlIdentitySchema.safeParse(
    record["originTurnId"],
  );
  if (!actionRequestId.success || !originTurnId.success) return null;

  const rawAction = record["action"];
  const action =
    typeof rawAction === "string" && rawAction.trim()
      ? rawAction.trim().slice(0, 4_096)
      : "invalid.action";
  const rawSchemaVersion = record["schemaVersion"];
  const schemaVersion =
    typeof rawSchemaVersion === "number" && Number.isInteger(rawSchemaVersion)
      ? rawSchemaVersion
      : 0;
  const actorId = optionalControlIdentitySchema.safeParse(record["actorId"]);
  return {
    schemaVersion,
    actorId: actorId.success ? actorId.data : "",
    originTurnId: originTurnId.data,
    taskId: "",
    stepId: "",
    actionRequestId: actionRequestId.data,
    action,
    params: {},
  };
}

export type ActionCardParams =
  | {
      readonly variant: "catalog";
      readonly service_slug: string;
      readonly requested_scopes: readonly string[];
      readonly via_node_id: string | null;
      readonly target_org_id: string | null;
    }
  | {
      readonly variant: "custom";
      readonly name: string;
      readonly endpoint_url: string;
      readonly auth_method: string;
      readonly auth_key_name: string;
      readonly via_node_id: string | null;
      readonly target_org_id: string | null;
    }
  | {
      readonly variant: "service_reauthorize";
      readonly user_service_id: string;
      readonly requested_scopes: readonly string[];
    }
  | {
      readonly variant: "key_create";
      readonly name: string;
      readonly platform: string;
      readonly allowed_service_ids: readonly string[];
    }
  | {
      readonly variant: "key_rotate";
      readonly key_id: string;
    }
  | { readonly variant: "unknown" };

export const actionDispositionSchema = z.enum([
  "completed",
  "declined",
  "failed",
  "cancelled",
  "expired",
]);

const userServiceResourceSchema = z
  .object({
    userService: z
      .object({ userServiceId: actionControlIdentitySchema })
      .strict(),
  })
  .strict();
const keyResourceSchema = z
  .object({
    key: z
      .object({
        keyId: actionControlIdentitySchema,
        // Present on key.bind_credential reports so the postcondition can
        // address GET /api-keys/{keyId}/bindings/by-service/{userServiceId}/authorization.
        // Other key verbs omit it.
        userServiceId: actionControlIdentitySchema.optional(),
      })
      .strict(),
  })
  .strict();
const nodeResourceSchema = z
  .object({ node: z.object({ nodeId: actionControlIdentitySchema }).strict() })
  .strict();
const serviceAccountResourceSchema = z
  .object({
    serviceAccount: z
      .object({ serviceAccountId: actionControlIdentitySchema })
      .strict(),
  })
  .strict();
const developerAppResourceSchema = z
  .object({
    developerApp: z.object({ clientId: actionControlIdentitySchema }).strict(),
  })
  .strict();
const endpointResourceSchema = z
  .object({
    endpoint: z.object({ endpointId: actionControlIdentitySchema }).strict(),
  })
  .strict();
// external_key.* reports MUST NOT reuse `key`: that variant addresses
// api_keys via /api/v1/api-keys/{id}/authorization, while these are
// user_api_keys rows read at /api/v1/api-keys/external/{id}/authorization.
const externalKeyResourceSchema = z
  .object({
    externalKey: z
      .object({ externalKeyId: actionControlIdentitySchema })
      .strict(),
  })
  .strict();
const deviceResourceSchema = z
  .object({
    device: z.object({ deviceId: actionControlIdentitySchema }).strict(),
  })
  .strict();

export const actionResourceSchema = z.union([
  userServiceResourceSchema,
  keyResourceSchema,
  endpointResourceSchema,
  externalKeyResourceSchema,
  nodeResourceSchema,
  serviceAccountResourceSchema,
  developerAppResourceSchema,
  deviceResourceSchema,
]);

export const actionReportSchema = z
  .object({
    actionRequestId: actionControlIdentitySchema,
    originTurnId: actionControlIdentitySchema,
    disposition: actionDispositionSchema,
    resource: actionResourceSchema.optional(),
  })
  .strict();

export const actionContinueBodySchema = z
  .object({
    type: z.literal("action.continue"),
    conversationId: actionControlIdentitySchema,
    clientRequestId: actionControlIdentitySchema,
    originTurnId: actionControlIdentitySchema,
    actions: z.array(actionReportSchema).min(1),
  })
  .strict()
  .superRefine((body, context) => {
    const ids = new Set<string>();
    for (const [index, report] of body.actions.entries()) {
      if (report.originTurnId !== body.originTurnId) {
        context.addIssue({
          code: "custom",
          path: ["actions", index, "originTurnId"],
          message: "Report origin must match the continuation origin",
        });
      }
      if (ids.has(report.actionRequestId)) {
        context.addIssue({
          code: "custom",
          path: ["actions", index, "actionRequestId"],
          message: "Duplicate action request id",
        });
      }
      ids.add(report.actionRequestId);
    }
  });

export const actionWakeBodySchema = z
  .object({
    type: z.literal("action.continue"),
    conversationId: actionControlIdentitySchema,
    clientRequestId: actionControlIdentitySchema,
    originTurnId: actionControlIdentitySchema.optional(),
    actions: z.tuple([]),
  })
  .strict();

export type ActionDisposition = z.infer<typeof actionDispositionSchema>;
export type ActionResource = z.infer<typeof actionResourceSchema>;
export type ActionReport = z.infer<typeof actionReportSchema>;
export type ActionContinueBody = z.infer<typeof actionContinueBodySchema>;
export type ActionWakeBody = z.infer<typeof actionWakeBodySchema>;
export type ActionReportActionLookup =
  ReadonlyMap<string, string> | Readonly<Record<string, string | undefined>>;
function matchesSecretValue(value: string): boolean {
  SECRET_VALUE.lastIndex = 0;
  return SECRET_VALUE.test(value);
}

function findSecretPath(
  value: unknown,
  path: ReadonlyArray<string | number> = [],
): ReadonlyArray<string | number> | null {
  if (typeof value === "string") {
    return matchesSecretValue(value) ? path : null;
  }
  if (Array.isArray(value)) {
    for (const [index, entry] of value.entries()) {
      const entryPath = findSecretPath(entry, [...path, index]);
      if (entryPath) return entryPath;
    }
    return null;
  }
  if (typeof value !== "object" || value === null) return null;

  for (const [key, entry] of Object.entries(value)) {
    // The current request schemas are `.strict()`, so undeclared members are
    // rejected before this key scan can fire. Keep the scan anyway as
    // defense-in-depth for any future schema widening or nested passthrough.
    if (FORBIDDEN_ACTION_KEY.test(key)) return [...path, key];
    const entryPath = findSecretPath(entry, [...path, key]);
    if (entryPath) return entryPath;
  }
  return null;
}

function actionForReport(
  actionRequestId: string,
  reportActions: ActionReportActionLookup,
): string | undefined {
  if (reportActions instanceof Map) {
    return reportActions.get(actionRequestId);
  }
  return (reportActions as Readonly<Record<string, string | undefined>>)[
    actionRequestId
  ];
}

function assertReportMatchesAction(
  report: ActionReport,
  action: string | undefined,
): void {
  if (report.disposition !== "completed") return;
  if (!report.resource) {
    throw new Error(
      "Completed action reports must include a resource reference",
    );
  }
  if (action?.startsWith("service.") && !("userService" in report.resource)) {
    throw new Error(
      `${action} completed reports must include resource.userService.userServiceId`,
    );
  }
  if (action?.startsWith("key.") && !("key" in report.resource)) {
    throw new Error(
      `${action} completed reports must include resource.key.keyId`,
    );
  }
  if (action?.startsWith("endpoint.") && !("endpoint" in report.resource)) {
    throw new Error(
      `${action} completed reports must include resource.endpoint.endpointId`,
    );
  }
  if (
    action?.startsWith("external_key.") &&
    !("externalKey" in report.resource)
  ) {
    throw new Error(
      `${action} completed reports must include resource.externalKey.externalKeyId`,
    );
  }
  if (
    action === "key.bind_credential" &&
    (!("key" in report.resource) || !report.resource.key.userServiceId)
  ) {
    throw new Error(
      "key.bind_credential completed reports must include resource.key.userServiceId",
    );
  }
}

function copyResource(resource: ActionResource): ActionResource {
  if ("userService" in resource) {
    return {
      userService: { userServiceId: resource.userService.userServiceId },
    };
  }
  if ("key" in resource) {
    return {
      key: {
        keyId: resource.key.keyId,
        ...(resource.key.userServiceId
          ? { userServiceId: resource.key.userServiceId }
          : {}),
      },
    };
  }
  if ("endpoint" in resource) {
    return { endpoint: { endpointId: resource.endpoint.endpointId } };
  }
  if ("externalKey" in resource) {
    return {
      externalKey: { externalKeyId: resource.externalKey.externalKeyId },
    };
  }
  if ("node" in resource) return { node: { nodeId: resource.node.nodeId } };
  if ("serviceAccount" in resource) {
    return {
      serviceAccount: {
        serviceAccountId: resource.serviceAccount.serviceAccountId,
      },
    };
  }
  if ("developerApp" in resource) {
    return { developerApp: { clientId: resource.developerApp.clientId } };
  }
  return { device: { deviceId: resource.device.deviceId } };
}

/** Build the strict DTO from an explicit allowlist of wire members. */
export function buildActionContinueBody(
  conversationId: string,
  clientRequestId: string,
  originTurnId: string,
  reports: readonly ActionReport[],
  reportActions: ActionReportActionLookup,
): ActionContinueBody {
  const actions = reports.map((report): ActionReport => {
    assertReportMatchesAction(
      report,
      actionForReport(report.actionRequestId, reportActions),
    );
    if (report.resource) {
      return {
        actionRequestId: report.actionRequestId,
        originTurnId: report.originTurnId,
        disposition: report.disposition,
        resource: copyResource(report.resource),
      };
    }
    return {
      actionRequestId: report.actionRequestId,
      originTurnId: report.originTurnId,
      disposition: report.disposition,
    };
  });
  return actionContinueBodySchema.parse({
    type: "action.continue",
    conversationId,
    clientRequestId,
    originTurnId,
    actions,
  });
}

/** Build the distinct out-of-band wake DTO with no action reports. */
export function buildActionWakeBody(
  conversationId: string,
  clientRequestId: string,
  originTurnId?: string,
): ActionWakeBody {
  return actionWakeBodySchema.parse({
    type: "action.continue",
    conversationId,
    clientRequestId,
    originTurnId,
    actions: [],
  });
}
