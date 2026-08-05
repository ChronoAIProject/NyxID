import { z } from "zod";

export const connectLinkStatusSchema = z.enum([
  "pending",
  "completed",
  "expired",
  "cancelled",
]);

export const connectMethodSchema = z.enum([
  "api_key",
  "oauth",
  "device_code",
  "none",
]);

export const connectLinkPreviewSchema = z.object({
  service_name: z.string().min(1),
  service_slug: z.string().min(1),
  label: z.string().nullable(),
  requested_by: z.string().nullable(),
  created_at: z.string().datetime(),
  expires_at: z.string().datetime(),
  status: connectLinkStatusSchema,
  connect_method: connectMethodSchema,
  auth_key_name: z.string().min(1),
  credential_mode: z.string().nullable(),
  has_platform_oauth_credentials: z.boolean(),
  requires_gateway_url: z.boolean(),
  api_key_url: z.string().url().nullable(),
  api_key_instructions: z.string().nullable(),
});

export const connectCredentialFormSchema = z.object({
  credential: z.string().trim().min(1, "Credential is required").max(16_384),
  endpoint_url: z
    .string()
    .trim()
    .refine(
      (value) => value.length === 0 || /^https?:\/\//i.test(value),
      "Enter an absolute HTTP(S) URL",
    ),
  oauth_client_id: z.string().trim().max(1_024),
  oauth_client_secret: z.string().trim().max(16_384),
});

export const connectOAuthFormSchema = z.object({
  endpoint_url: z
    .string()
    .trim()
    .refine(
      (value) => value.length === 0 || /^https?:\/\//i.test(value),
      "Enter an absolute HTTP(S) URL",
    ),
  oauth_client_id: z.string().trim().max(1_024),
  oauth_client_secret: z.string().trim().max(16_384),
});

export const completeConnectLinkResponseSchema = z.object({
  id: z.string().uuid(),
  status: z.enum([
    "completed",
    "oauth_required",
    "device_code_required",
  ]),
  service_slug: z.string().min(1),
  user_service_id: z.string().nullable().optional(),
  authorization_url: z.string().url().nullable().optional(),
  device_user_code: z.string().nullable().optional(),
  device_verification_uri: z.string().url().nullable().optional(),
  device_state: z.string().nullable().optional(),
  device_interval: z.number().int().positive().nullable().optional(),
  device_status: z.string().nullable().optional(),
  callback_url: z.string().url().nullable().optional(),
});

export const connectLinkStatusResponseSchema = z.object({
  id: z.string().uuid(),
  status: connectLinkStatusSchema,
  service_name: z.string(),
  service_slug: z.string(),
  expires_at: z.string().datetime(),
  completed_at: z.string().datetime().nullable().optional(),
  connected_service: z
    .object({ id: z.string(), slug: z.string() })
    .nullable()
    .optional(),
  callback_url: z.string().url().nullable().optional(),
});

export type ConnectLinkPreview = z.infer<typeof connectLinkPreviewSchema>;
export type ConnectCredentialForm = z.infer<typeof connectCredentialFormSchema>;
export type ConnectOAuthForm = z.infer<typeof connectOAuthFormSchema>;
export interface CompleteConnectLinkInput {
  readonly credential?: string;
  readonly endpoint_url?: string;
  readonly oauth_client_id?: string;
  readonly oauth_client_secret?: string;
  readonly device_state?: string;
}
export type CompleteConnectLinkResponse = z.infer<
  typeof completeConnectLinkResponseSchema
>;
export type ConnectLinkStatusResponse = z.infer<
  typeof connectLinkStatusResponseSchema
>;

export function validateConnectCredentialForm(
  values: ConnectCredentialForm,
  requiresGatewayUrl: boolean,
): string | null {
  if (requiresGatewayUrl && values.endpoint_url.length === 0) {
    return "Endpoint URL is required for this service";
  }
  const hasClientId = values.oauth_client_id.length > 0;
  const hasClientSecret = values.oauth_client_secret.length > 0;
  if (hasClientId !== hasClientSecret) {
    return "OAuth client ID and secret must be supplied together";
  }
  return null;
}

export function validateConnectOAuthForm(
  values: ConnectOAuthForm,
  requiresGatewayUrl: boolean,
  requiresClientCredentials: boolean,
): string | null {
  if (requiresGatewayUrl && values.endpoint_url.length === 0) {
    return "Endpoint URL is required for this service";
  }
  if (
    requiresClientCredentials &&
    (values.oauth_client_id.length === 0 || values.oauth_client_secret.length === 0)
  ) {
    return "OAuth client ID and secret are required for this service";
  }
  const hasClientId = values.oauth_client_id.length > 0;
  const hasClientSecret = values.oauth_client_secret.length > 0;
  if (hasClientId !== hasClientSecret) {
    return "OAuth client ID and secret must be supplied together";
  }
  return null;
}
