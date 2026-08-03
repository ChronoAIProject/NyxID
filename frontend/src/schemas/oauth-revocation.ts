import { z } from "zod";

export const GRANT_CASCADE_ERROR_CODE = 11500;

export function grantCascadeCaveat(providerName: string): string {
  return `If this ${providerName} account is also connected under another NyxID account or organization, those connections will lose access too. If any listed service uses a different ${providerName} account, it will be removed from NyxID but keep its ${providerName}-side access.`;
}

export function grantRevocationDescription(providerName: string): string {
  return `This de-authorizes NyxID from your ${providerName} account; you'll see the consent screen if you reconnect.`;
}

export const grantCascadeSiblingSchema = z.object({
  user_service_id: z.string(),
  name: z.string().min(1),
  slug: z.string(),
});

export const grantCascadeDetailsSchema = z.object({
  provider_slug: z.string().min(1),
  provider_name: z.string().min(1),
  revokes_grant: z.boolean(),
  siblings: z.array(grantCascadeSiblingSchema),
  unaffected_other_app: z.array(grantCascadeSiblingSchema),
  token_scope_available: z.boolean(),
});

export const grantCascadeErrorResponseSchema = z.object({
  error_code: z.literal(GRANT_CASCADE_ERROR_CODE),
  details: grantCascadeDetailsSchema,
});

export type GrantCascadeDetails = z.infer<typeof grantCascadeDetailsSchema>;

export function parseGrantCascadeDetails(
  errorResponse: unknown,
): GrantCascadeDetails | null {
  const parsed = grantCascadeErrorResponseSchema.safeParse(errorResponse);
  return parsed.success ? parsed.data.details : null;
}
