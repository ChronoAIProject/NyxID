import { z } from "zod";

export const AUTH_TYPES = [
  "none",
  "api_key",
  "oauth2",
  "basic",
  "bearer",
  "oidc",
] as const;

export type AuthType = (typeof AUTH_TYPES)[number];

export const SERVICE_TYPES = ["http", "ssh"] as const;

export type ServiceType = (typeof SERVICE_TYPES)[number];

export const SERVICE_CATEGORIES = [
  "provider",
  "connection",
  "internal",
] as const;

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
        message: "At least one SSH principal is required when certificate auth is enabled",
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
    base_url: optionalString,
    auth_type: z.enum(AUTH_TYPES).optional(),
    service_category: z.enum(SERVICE_CATEGORIES).optional(),
    host: optionalString,
    port: optionalString,
    certificate_auth_enabled: z.boolean().optional(),
    certificate_ttl_minutes: optionalString,
    allowed_principals: optionalString,
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

export const updateServiceSchema = z
  .object({
    service_type: z.enum(SERVICE_TYPES),
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
    identity_propagation_mode: z
      .enum(IDENTITY_PROPAGATION_MODES)
      .optional(),
    identity_include_user_id: z.boolean().optional(),
    identity_include_email: z.boolean().optional(),
    identity_include_name: z.boolean().optional(),
    identity_jwt_audience: z.string().max(500).optional().or(z.literal("")),
    inject_delegation_token: z.boolean().optional(),
    delegation_token_scope: z
      .string()
      .max(200, "Scope must be at most 200 characters")
      .optional()
      .or(z.literal("")),
    host: optionalString,
    port: optionalString,
    certificate_auth_enabled: z.boolean().optional(),
    certificate_ttl_minutes: optionalString,
    allowed_principals: optionalString,
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

// SEC-1: Restrict redirect URIs to http/https schemes only
export const redirectUriSchema = z
  .string()
  .min(1, "URI is required")
  .url("Must be a valid URL")
  .refine(
    (val) => val.startsWith("https://") || val.startsWith("http://"),
    "URI must use https:// or http://"
  );
