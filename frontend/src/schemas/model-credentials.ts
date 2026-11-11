import { z } from "zod";

export const credentialKinds = {
  "gcp-service-account": "Google Cloud service account (Vertex AI)",
  openai: "OpenAI API key",
  anthropic: "Anthropic API key",
} as const;
export type CredentialKind = keyof typeof credentialKinds;
export const credentialSchema = z.object({
  id: z.string(),
  name: z.string(),
  kind: z.enum(["gcp-service-account", "openai", "anthropic"]).nullable(),
  enabled: z.boolean(),
  created_at: z.string(),
  updated_at: z.string(),
  references: z.array(
    z.object({
      model_id: z.string(),
      public_id: z.string(),
      display_name: z.string(),
      route_id: z.string().nullable().optional(),
      enabled: z.boolean(),
    }),
  ),
});
export type ModelCredential = z.infer<typeof credentialSchema>;
export const credentialPageSchema = z.object({
  records: z.array(credentialSchema),
  next_after: z.string().nullable(),
});

/** Matches token.rs's closed service-account shape; RSA parsing stays server-side. */
export function credentialError(
  raw: string,
  kind: CredentialKind | null,
): string | undefined {
  if (!raw || new TextEncoder().encode(raw).length > 65536)
    return "Enter a credential of at most 64 KiB.";
  if (
    kind !== "gcp-service-account" &&
    !(kind === null && raw.trimStart().startsWith("{"))
  ) {
    return raw.length <= 8192 && /^[A-Za-z0-9._~+/-]+=*$/.test(raw)
      ? undefined
      : "Enter a valid provider API key.";
  }
  const error =
    "Choose a Google Cloud service_account JSON key with the standard Google endpoints and required fields. External-account credentials are not supported.";
  try {
    const key = JSON.parse(raw) as Record<string, unknown>;
    const required = [
      "type",
      "project_id",
      "private_key_id",
      "private_key",
      "client_email",
      "client_id",
    ];
    const optional = [
      "auth_uri",
      "token_uri",
      "auth_provider_x509_cert_url",
      "client_x509_cert_url",
      "universe_domain",
    ];
    if (
      !key ||
      typeof key !== "object" ||
      Array.isArray(key) ||
      Object.keys(key).some((k) => ![...required, ...optional].includes(k)) ||
      required.some((k) => typeof key[k] !== "string" || !key[k])
    )
      return error;
    if (
      key.type !== "service_account" ||
      new TextEncoder().encode(String(key.private_key_id)).length > 256 ||
      new TextEncoder().encode(String(key.client_email)).length > 320 ||
      !String(key.client_email).endsWith(".iam.gserviceaccount.com") ||
      !/^-----BEGIN (?:RSA )?PRIVATE KEY-----[\s\S]+-----END (?:RSA )?PRIVATE KEY-----\s*$/.test(
        String(key.private_key),
      )
    )
      return error;
    const fixed: Record<string, string> = {
      token_uri: "https://oauth2.googleapis.com/token",
      auth_uri: "https://accounts.google.com/o/oauth2/auth",
      auth_provider_x509_cert_url: "https://www.googleapis.com/oauth2/v1/certs",
      universe_domain: "googleapis.com",
    };
    if (
      Object.entries(fixed).some(
        ([field, value]) => key[field] != null && key[field] !== value,
      )
    )
      return error;
    if (
      key.client_x509_cert_url != null &&
      (typeof key.client_x509_cert_url !== "string" ||
        !key.client_x509_cert_url.startsWith(
          "https://www.googleapis.com/robot/v1/metadata/x509/",
        ))
    )
      return error;
    return undefined;
  } catch {
    return error;
  }
}
