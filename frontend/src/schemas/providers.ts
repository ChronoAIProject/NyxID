import { z } from "zod";

export const CREDENTIAL_MODES = ["admin", "user", "both"] as const;

export type CredentialModeType = (typeof CREDENTIAL_MODES)[number];

export const userCredentialsSchema = z.object({
  client_id: z
    .string()
    .min(1, "Client ID is required")
    .max(500, "Client ID must be at most 500 characters"),
  client_secret: z
    .string()
    .max(2000, "Client Secret must be at most 2000 characters")
    .optional()
    .or(z.literal("")),
  label: z
    .string()
    .max(200, "Label must be at most 200 characters")
    .optional()
    .or(z.literal("")),
});

export type UserCredentialsFormData = z.infer<typeof userCredentialsSchema>;

export const connectApiKeySchema = z.object({
  api_key: z
    .string()
    .min(1, "API key is required")
    .max(8192, "API key must be at most 8192 characters"),
  label: z.string().max(200, "Label must be at most 200 characters").optional(),
});

export type ConnectApiKeyFormData = z.infer<typeof connectApiKeySchema>;

export const PROVIDER_TYPES = ["oauth2", "api_key", "device_code"] as const;

export type ProviderType = (typeof PROVIDER_TYPES)[number];

const SLUG_PATTERN = /^[a-z0-9][a-z0-9-]*[a-z0-9]$/;

const baseProviderFields = {
  name: z
    .string()
    .min(2, "Name must be at least 2 characters")
    .max(100, "Name must be at most 100 characters"),
  slug: z
    .string()
    .min(2, "Slug must be at least 2 characters")
    .max(50, "Slug must be at most 50 characters")
    .regex(
      SLUG_PATTERN,
      "Slug must contain only lowercase letters, digits, and hyphens (no leading/trailing hyphens)",
    ),
  description: z
    .string()
    .max(500, "Description must be at most 500 characters")
    .optional()
    .or(z.literal("")),
  provider_type: z.enum(PROVIDER_TYPES),
  authorization_url: z
    .string()
    .url("Must be a valid URL")
    .optional()
    .or(z.literal("")),
  token_url: z.string().url("Must be a valid URL").optional().or(z.literal("")),
  revocation_url: z
    .string()
    .url("Must be a valid URL")
    .optional()
    .or(z.literal("")),
  default_scopes: z
    .string()
    .max(2000, "Scopes must be at most 2000 characters")
    .optional()
    .or(z.literal("")),
  credential_mode: z.enum(CREDENTIAL_MODES).optional(),
  supports_pkce: z.boolean().optional(),
  device_code_url: z
    .string()
    .url("Must be a valid URL")
    .optional()
    .or(z.literal("")),
  device_token_url: z
    .string()
    .url("Must be a valid URL")
    .optional()
    .or(z.literal("")),
  device_verification_url: z
    .string()
    .url("Must be a valid URL")
    .optional()
    .or(z.literal("")),
  hosted_callback_url: z
    .string()
    .url("Must be a valid URL")
    .optional()
    .or(z.literal("")),
  api_key_instructions: z
    .string()
    .max(2000, "Instructions must be at most 2000 characters")
    .optional()
    .or(z.literal("")),
  api_key_url: z
    .string()
    .url("Must be a valid URL")
    .optional()
    .or(z.literal("")),
  icon_url: z.string().url("Must be a valid URL").optional().or(z.literal("")),
  documentation_url: z
    .string()
    .url("Must be a valid URL")
    .optional()
    .or(z.literal("")),
  token_endpoint_auth_method: z
    .enum(["client_secret_post", "client_secret_basic"])
    .optional(),
  extra_auth_params: z.record(z.string(), z.string()).optional(),
  device_code_format: z.enum(["rfc8628", "openai"]).optional(),
  client_id_param_name: z
    .string()
    .max(100, "Param name must be at most 100 characters")
    .optional()
    .or(z.literal("")),
} as const;

export const createProviderSchema = z
  .object({
    ...baseProviderFields,
    client_id: z.string().optional().or(z.literal("")),
    client_secret: z.string().optional().or(z.literal("")),
  })
  .superRefine((data, ctx) => {
    const mode = data.credential_mode ?? "admin";
    if (data.provider_type === "oauth2") {
      if (!data.authorization_url) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          message: "Authorization URL is required for OAuth2 providers",
          path: ["authorization_url"],
        });
      }
      if (!data.token_url) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          message: "Token URL is required for OAuth2 providers",
          path: ["token_url"],
        });
      }
      if (mode === "admin") {
        if (!data.client_id) {
          ctx.addIssue({
            code: z.ZodIssueCode.custom,
            message: "Client ID is required for OAuth2 providers in admin mode",
            path: ["client_id"],
          });
        }
        if (!data.client_secret) {
          ctx.addIssue({
            code: z.ZodIssueCode.custom,
            message:
              "Client Secret is required for OAuth2 providers in admin mode",
            path: ["client_secret"],
          });
        }
      } else if (!!data.client_id !== !!data.client_secret) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          message:
            "Admin fallback OAuth2 credentials must include both Client ID and Client Secret",
          path: [data.client_id ? "client_secret" : "client_id"],
        });
      }
    }
    if (data.provider_type === "device_code") {
      if (!data.authorization_url) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          message: "Authorization URL is required for device code providers",
          path: ["authorization_url"],
        });
      }
      if (!data.token_url) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          message: "Token URL is required for device code providers",
          path: ["token_url"],
        });
      }
      if (mode === "admin" && !data.client_id) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          message:
            "Client ID is required for device code providers in admin mode",
          path: ["client_id"],
        });
      }
      if (data.client_secret && !data.client_id) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          message: "Client ID is required when Client Secret is set",
          path: ["client_id"],
        });
      }
      if (!data.device_code_url) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          message: "Device Code URL is required for device code providers",
          path: ["device_code_url"],
        });
      }
      if (!data.device_token_url) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          message: "Device Token URL is required for device code providers",
          path: ["device_token_url"],
        });
      }
    }
  });

export type CreateProviderFormData = z.infer<typeof createProviderSchema>;

export const updateProviderSchema = z
  .object({
    ...baseProviderFields,
    is_active: z.boolean().optional(),
    client_id: z.string().optional().or(z.literal("")),
    client_secret: z.string().optional().or(z.literal("")),
  })
  .superRefine((data, ctx) => {
    if (data.provider_type === "oauth2") {
      if (!data.authorization_url) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          message: "Authorization URL is required for OAuth2 providers",
          path: ["authorization_url"],
        });
      }
      if (!data.token_url) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          message: "Token URL is required for OAuth2 providers",
          path: ["token_url"],
        });
      }
    }
    if (data.provider_type === "device_code") {
      if (!data.authorization_url) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          message: "Authorization URL is required for device code providers",
          path: ["authorization_url"],
        });
      }
      if (!data.token_url) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          message: "Token URL is required for device code providers",
          path: ["token_url"],
        });
      }
    }
    // Note: device_code_url and device_token_url are optional on update (blank = keep current)
  });

export type UpdateProviderFormData = z.infer<typeof updateProviderSchema>;
