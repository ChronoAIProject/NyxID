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

const SECRET_VALUE = /(Bearer\s+)[A-Za-z0-9._~+/-]+|nyx(?:id)?_[A-Za-z0-9_-]{8,}/gi;
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
    requestedScopes: z.array(z.string().max(256)).optional().default([]),
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

export const assistantActionParamsSchema = z
  .object({
    catalogService: catalogServiceActionParamsSchema.optional(),
    customService: customServiceActionParamsSchema.optional(),
  })
  .strict()
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
  .object({ key: z.object({ keyId: actionControlIdentitySchema }).strict() })
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
const deviceResourceSchema = z
  .object({
    device: z.object({ deviceId: actionControlIdentitySchema }).strict(),
  })
  .strict();

export const actionResourceSchema = z.union([
  userServiceResourceSchema,
  keyResourceSchema,
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

export type ActionDisposition = z.infer<typeof actionDispositionSchema>;
export type ActionResource = z.infer<typeof actionResourceSchema>;
export type ActionReport = z.infer<typeof actionReportSchema>;
export type ActionContinueBody = z.infer<typeof actionContinueBodySchema>;
export type ActionReportActionLookup =
  | ReadonlyMap<string, string>
  | Readonly<Record<string, string | undefined>>;

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
  if (
    action === "service.connect" &&
    (!report.resource || !("userService" in report.resource))
  ) {
    throw new Error(
      "service.connect completed reports must include resource.userService.userServiceId",
    );
  }
  if (!report.resource) {
    throw new Error("Completed action reports must include a resource reference");
  }
}

function copyResource(resource: ActionResource): ActionResource {
  if ("userService" in resource) {
    return {
      userService: { userServiceId: resource.userService.userServiceId },
    };
  }
  if ("key" in resource) return { key: { keyId: resource.key.keyId } };
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
    clientRequestId,
    originTurnId,
    actions,
  });
}
