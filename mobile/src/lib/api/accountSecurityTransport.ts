import { z } from "zod";
import type { RequestOptions } from "./authenticatedTransport";

const required = z.string().trim().min(1);
const credentialOptions = {
  retryOnAuthFailure: false,
  invalidateSessionOnAuthFailure: false,
} as const;
export const disableMfaSchema = z.object({
  code: required.max(128),
  password: z.string().min(1).optional(),
});
const password = z.string().refine((value) => {
  let length = 0;
  for (const character of value) {
    const point = character.codePointAt(0) ?? 0;
    length += point <= 0x7f ? 1 : point <= 0x7ff ? 2 : point <= 0xffff ? 3 : 4;
  }
  return length >= 8 && length <= 128;
}, "New password must be 8–128 UTF-8 bytes");
export const changePasswordSchema = z
  .object({
    current_password: z.string().min(1, "Current password is required"),
    new_password: password,
    confirmation: z.string(),
  })
  .refine(
    (value) => value.new_password === value.confirmation,
    "Passwords do not match",
  );
export const securityPreferencesSchema = z.object({
  skip_mfa_for_approved_sign_ins: z.boolean(),
});
const consentSchema = z.object({
  id: required,
  client_id: required,
  client_name: z.string(),
  scopes: z.string(),
  allow_all_services: z.boolean(),
  allowed_service_ids: z.array(z.string()),
  allowed_services: z.array(
    z.object({ id: z.string(), label: z.string(), deleted: z.boolean() }),
  ),
  legacy_unrestricted: z.boolean(),
  granted_at: z.string(),
  expires_at: z.string().nullable(),
});
const bindingSchema = z.object({
  binding_hash: required,
  client_id: required,
  client_name: z.string().nullable(),
  external_subject: z
    .object({
      platform: z.string(),
      tenant: z.string().optional(),
      external_user_id: z.string(),
    })
    .nullable(),
  scopes: z.array(z.string()),
  created_at: z.string(),
  last_used_at: z.string().nullable(),
});
export const organizationSchema = z.object({
  id: required,
  slug: z.string(),
  display_name: z.string().nullable(),
  your_role: z.enum(["owner", "admin", "member", "viewer"]),
});
export type AccountConsent = z.infer<typeof consentSchema>;
export type BrokerAuthorization = z.infer<typeof bindingSchema>;
export type AccountOrganization = z.infer<typeof organizationSchema>;
type Request = (path: string, options?: RequestOptions) => Promise<unknown>;
// Each call is fenced to the account shown in the UI, including transport retries.
export function createAccountSecurityApi(request: Request) {
  const options = (
    accountId: string,
    extra: RequestOptions = {},
  ): RequestOptions => ({
    expectedAccountId: required.parse(accountId),
    ...extra,
  });
  return {
    disableMfa: (input: z.infer<typeof disableMfaSchema>) =>
      request("/auth/mfa/disable", {
        method: "POST",
        body: disableMfaSchema.parse(input),
        ...credentialOptions,
      }),
    changePassword: (
      accountId: string,
      input: z.infer<typeof changePasswordSchema>,
    ) => {
      const { confirmation: _, ...body } = changePasswordSchema.parse(input);
      return request(
        "/auth/password/change",
        options(accountId, { method: "POST", body, ...credentialOptions }),
      );
    },
    async saveSecurityPreferences(
      accountId: string,
      input: z.infer<typeof securityPreferencesSchema>,
    ) {
      return securityPreferencesSchema.parse(
        await request(
          "/users/me/security",
          options(accountId, {
            method: "PUT",
            body: securityPreferencesSchema.parse(input),
            ...credentialOptions,
          }),
        ),
      );
    },
    async consents(accountId: string, signal?: AbortSignal) {
      return z
        .object({ consents: z.array(consentSchema) })
        .parse(
          await request("/users/me/consents", options(accountId, { signal })),
        ).consents;
    },
    revokeConsent: (accountId: string, clientId: string) =>
      request(
        `/users/me/consents/${encodeURIComponent(required.parse(clientId))}`,
        options(accountId, { method: "DELETE" }),
      ),
    async brokerBindings(accountId: string, signal?: AbortSignal) {
      return z
        .object({ bindings: z.array(bindingSchema) })
        .parse(
          await request(
            "/users/me/broker-bindings",
            options(accountId, { signal }),
          ),
        ).bindings;
    },
    revokeBrokerBinding: (accountId: string, hash: string) =>
      request(
        `/users/me/broker-bindings/${encodeURIComponent(required.parse(hash))}`,
        options(accountId, { method: "DELETE" }),
      ),
    async organizations(accountId: string, signal?: AbortSignal) {
      return z
        .object({ orgs: z.array(organizationSchema) })
        .parse(await request("/orgs", options(accountId, { signal }))).orgs;
    },
    async createOrganization(accountId: string, name: string) {
      const body = z
        .object({ display_name: required.max(128) })
        .parse({ display_name: name });
      return organizationSchema.parse(
        await request("/orgs", options(accountId, { method: "POST", body })),
      );
    },
    async joinOrganization(accountId: string, nonce: string) {
      return z
        .object({ org_id: required, role: organizationSchema.shape.your_role })
        .parse(
          await request(
            `/orgs/join/${encodeURIComponent(required.parse(nonce))}`,
            options(accountId, { method: "POST" }),
          ),
        );
    },
  };
}
