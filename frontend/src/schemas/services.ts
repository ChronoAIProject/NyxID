import { z } from "zod";
import {
  defaultRequestHeaderListSchema,
  defaultRequestHeaderSchema,
  defaultRequestHeaderUpdateSchema,
} from "./default-request-headers";

export {
  defaultRequestHeaderListSchema,
  defaultRequestHeaderSchema,
  defaultRequestHeaderUpdateSchema,
} from "./default-request-headers";
export type {
  DefaultRequestHeader,
  DefaultRequestHeaderList,
  DefaultRequestHeaderUpdate,
} from "./default-request-headers";

export const AUTH_TYPES = [
  "none",
  "api_key",
  "oauth2",
  "basic",
  "bearer",
  "bot_bearer",
  "body",
  "path",
  "oidc",
] as const;

export type AuthType = (typeof AUTH_TYPES)[number];

export const SERVICE_TYPES = ["http", "ssh"] as const;
export const SSH_AUTH_MODES = ["cert", "node_key", "proxy_only"] as const;
export const sshAuthModeSchema = z.enum(SSH_AUTH_MODES);
export type SshAuthMode = z.infer<typeof sshAuthModeSchema>;

export type ServiceType = (typeof SERVICE_TYPES)[number];

export const SERVICE_CATEGORIES = [
  "provider",
  "connection",
  "internal",
] as const;

export const VISIBILITY_OPTIONS = ["public", "private"] as const;

export type Visibility = (typeof VISIBILITY_OPTIONS)[number];

export type ServiceCategory = (typeof SERVICE_CATEGORIES)[number];

const optionalString = z.string().optional().or(z.literal(""));
const urlField = z.string().url("Must be a valid URL");

export const sshServiceConfigSchema = z
  .object({
    host: z
      .string()
      .trim()
      .min(1, "Host is required")
      .max(255, "Host must be at most 255 characters"),
    port: z
      .string()
      .min(1, "Port is required")
      .refine((value) => {
        const port = Number(value);
        return Number.isInteger(port) && port >= 1 && port <= 65535;
      }, "Port must be an integer between 1 and 65535"),
    certificate_auth_enabled: z.boolean(),
    certificate_ttl_minutes: z
      .string()
      .min(1, "Certificate TTL is required")
      .refine((value) => {
        const ttl = Number(value);
        return Number.isInteger(ttl) && ttl >= 15 && ttl <= 60;
      }, "Certificate TTL must be an integer between 15 and 60 minutes"),
    allowed_principals: z
      .string()
      .max(500, "Allowed principals must be at most 500 characters"),
  })
  .superRefine((value, ctx) => {
    if (!value.certificate_auth_enabled) {
      return;
    }

    const principals = (value.allowed_principals ?? "")
      .split(/[\n,]/)
      .map((principal) => principal.trim())
      .filter(Boolean);

    if (principals.length === 0) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        path: ["allowed_principals"],
        message:
          "At least one SSH principal is required when certificate auth is enabled",
      });
    }
  });

export type SshServiceConfigFormData = z.infer<typeof sshServiceConfigSchema>;

function applySshFieldValidation(
  value: {
    readonly host?: string;
    readonly port?: string;
    readonly certificate_auth_enabled?: boolean;
    readonly certificate_ttl_minutes?: string;
    readonly allowed_principals?: string;
  },
  ctx: z.RefinementCtx,
) {
  const sshResult = sshServiceConfigSchema.safeParse({
    host: value.host ?? "",
    port: value.port ?? "",
    certificate_auth_enabled: value.certificate_auth_enabled ?? false,
    certificate_ttl_minutes: value.certificate_ttl_minutes ?? "30",
    allowed_principals: value.allowed_principals ?? "",
  });

  if (!sshResult.success) {
    for (const issue of sshResult.error.issues) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        path: issue.path,
        message: issue.message,
      });
    }
  }
}

// CR-6: Aligned with backend max length of 200 characters
export const createServiceSchema = z
  .object({
    name: z
      .string()
      .min(1, "Name is required")
      .max(200, "Name must be at most 200 characters"),
    description: z
      .string()
      .max(500, "Description must be at most 500 characters")
      .optional(),
    service_type: z.enum(SERVICE_TYPES),
    visibility: z.enum(VISIBILITY_OPTIONS).optional(),
    base_url: optionalString,
    auth_type: z.enum(AUTH_TYPES).optional(),
    /// JSON body key for `body` auth. Required when `auth_type === "body"`.
    auth_key_name: optionalString,
    service_category: z.enum(SERVICE_CATEGORIES).optional(),
    host: optionalString,
    port: optionalString,
    certificate_auth_enabled: z.boolean().optional(),
    certificate_ttl_minutes: optionalString,
    allowed_principals: optionalString,
    // NyxID#356: optional seed list for `default_request_headers`.
    default_request_headers: defaultRequestHeaderListSchema.optional(),
  })
  .superRefine((value, ctx) => {
    if (value.service_type === "http") {
      if (!value.base_url) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          path: ["base_url"],
          message: "Base URL is required",
        });
      } else if (!urlField.safeParse(value.base_url).success) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          path: ["base_url"],
          message: "Must be a valid URL",
        });
      }

      if (!value.auth_type) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          path: ["auth_type"],
          message: "Auth type is required",
        });
      }

      // `body` auth needs to know which JSON field to inject the credential
      // into (e.g. "app_secret" for Lark tenant token exchange).
      if (value.auth_type === "body" && !value.auth_key_name) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          path: ["auth_key_name"],
          message:
            "Field name is required for Body auth (e.g. 'app_secret' for Lark tenant token)",
        });
      }
      return;
    }

    applySshFieldValidation(
      {
        host: value.host,
        port: value.port,
        certificate_auth_enabled: value.certificate_auth_enabled,
        certificate_ttl_minutes: value.certificate_ttl_minutes,
        allowed_principals: value.allowed_principals,
      },
      ctx,
    );
  });

export type CreateServiceFormData = z.infer<typeof createServiceSchema>;

export const IDENTITY_PROPAGATION_MODES = [
  "none",
  "headers",
  "jwt",
  "both",
] as const;

export type IdentityPropagationMode =
  (typeof IDENTITY_PROPAGATION_MODES)[number];

export const wsFrameTriggerSchema = z.union([
  z.literal("first_frame_from_downstream"),
  z.object({
    json_field_equals: z.object({
      path: z.string().min(1, "JSON path is required"),
      value: z.unknown(),
    }),
  }),
  z.object({
    frame_index_from_downstream: z.object({
      index: z.number().int().min(0),
    }),
  }),
]);

export const wsFrameInjectionSchema = z.object({
  trigger: wsFrameTriggerSchema,
  template: z
    .string()
    .max(4096, "Template must be at most 4096 characters"),
  frame_kind: z.enum(["text", "binary"]),
  consume_trigger: z.boolean(),
  direction: z.enum(["downstream", "upstream"]),
});

export const wsFrameInjectionsSchema = z
  .array(wsFrameInjectionSchema)
  .max(4, "At most 4 WebSocket auth-frame rules are allowed");

export type WsFrameTrigger = z.infer<typeof wsFrameTriggerSchema>;
export type WsFrameInjection = z.infer<typeof wsFrameInjectionSchema>;

export const updateServiceSchema = z
  .object({
    service_type: z.enum(SERVICE_TYPES),
    visibility: z.enum(VISIBILITY_OPTIONS).optional(),
    name: z
      .string()
      .min(1, "Name is required")
      .max(200, "Name must be at most 200 characters"),
    description: z
      .string()
      .max(500, "Description must be at most 500 characters")
      .optional()
      .or(z.literal("")),
    base_url: optionalString,
    openapi_spec_url: z
      .string()
      .url("Must be a valid URL")
      .optional()
      .or(z.literal("")),
    asyncapi_spec_url: z
      .string()
      .url("Must be a valid URL")
      .optional()
      .or(z.literal("")),
    identity_propagation_mode: z.enum(IDENTITY_PROPAGATION_MODES).optional(),
    identity_include_user_id: z.boolean().optional(),
    identity_include_email: z.boolean().optional(),
    identity_include_name: z.boolean().optional(),
    identity_jwt_audience: z.string().max(500).optional().or(z.literal("")),
    forward_access_token: z.boolean().optional(),
    inject_delegation_token: z.boolean().optional(),
    delegation_token_scope: z
      .string()
      .max(200, "Scope must be at most 200 characters")
      .optional()
      .or(z.literal("")),
    // Rich metadata
    homepage_url: z.string().url("Must be a valid URL").optional().or(z.literal("")),
    repository_url: z.string().url("Must be a valid URL").optional().or(z.literal("")),
    issues_url: z.string().url("Must be a valid URL").optional().or(z.literal("")),
    auth_notes: z.string().max(4096, "Must be at most 4096 characters").optional().or(z.literal("")),
    known_limitations: z.string().max(4096, "Must be at most 4096 characters").optional().or(z.literal("")),
    required_permissions: z.string().max(2000, "Must be at most 2000 characters").optional().or(z.literal("")),
    examples_url: z.string().url("Must be a valid URL").optional().or(z.literal("")),
    recommended_skills: z.string().max(2000, "Must be at most 2000 characters").optional().or(z.literal("")),
    // Developer app scoping (admin-only, private services)
    developer_app_ids: z.array(z.string()).optional(),
    supports_proxy_read: z.boolean().optional(),
    supports_proxy_write: z.boolean().optional(),
    supports_proxy_binary_upload: z.boolean().optional(),
    supports_direct_downstream_auth: z.boolean().optional(),
    supports_authoring_via_nyx: z.boolean().optional(),
    supports_websocket: z.boolean().optional(),
    supports_streaming: z.boolean().optional(),
    host: optionalString,
    port: optionalString,
    certificate_auth_enabled: z.boolean().optional(),
    certificate_ttl_minutes: optionalString,
    allowed_principals: optionalString,
    /// NyxID#356: admin-facing default request headers for this service.
    /// `undefined` leaves the value unchanged, `null` clears, an array
    /// replaces. Matches the backend `Option<Option<Vec<...>>>` semantics.
    default_request_headers: defaultRequestHeaderUpdateSchema,
    ws_frame_injections: wsFrameInjectionsSchema.optional(),
  })
  .superRefine((value, ctx) => {
    if (value.service_type === "http") {
      if (!value.base_url) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          path: ["base_url"],
          message: "Base URL is required",
        });
      } else if (!urlField.safeParse(value.base_url).success) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          path: ["base_url"],
          message: "Must be a valid URL",
        });
      }
      return;
    }

    applySshFieldValidation(
      {
        host: value.host,
        port: value.port,
        certificate_auth_enabled: value.certificate_auth_enabled,
        certificate_ttl_minutes: value.certificate_ttl_minutes,
        allowed_principals: value.allowed_principals,
      },
      ctx,
    );
  });

export type UpdateServiceFormData = z.infer<typeof updateServiceSchema>;

/**
 * Shape fragment for NyxID#356 on `ServiceResponse` / `DownstreamService`
 * payloads. Admin and user responses both carry this field when present.
 * Kept as a separate schema (rather than a monolithic response schema)
 * so callers can extend their existing `z.object` / TS interfaces
 * without rewriting the whole thing.
 */
export const serviceResponseDefaultHeadersSchema = z.object({
  default_request_headers: z.array(defaultRequestHeaderSchema).optional(),
});

export type ServiceResponseDefaultHeaders = z.infer<
  typeof serviceResponseDefaultHeadersSchema
>;

// SEC-1: Restrict redirect URIs to http/https schemes only
export const redirectUriSchema = z
  .string()
  .min(1, "URI is required")
  .url("Must be a valid URL")
  .refine(
    (val) => val.startsWith("https://") || val.startsWith("http://"),
    "URI must use https:// or http://",
  );
