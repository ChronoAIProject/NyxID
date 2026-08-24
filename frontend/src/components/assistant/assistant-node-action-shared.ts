import { z } from "zod";
import { api } from "@/lib/api-client";
import { assertSecretFreeReadBack } from "./assistant-action-dialog-utils";

export const oneTimeMaterialSchema = z
  .enum(["delivered", "unavailable"])
  .optional();

export const assistantNodeEffectResponseSchema = z
  .object({
    resource: z.object({ nodeId: z.string().min(1) }).strict(),
    replayed: z.boolean(),
    requestedAt: z.string(),
    oneTimeMaterial: oneTimeMaterialSchema,
    registrationToken: z.string().min(1).optional(),
    authToken: z.string().min(1).optional(),
    signingSecret: z.string().min(1).optional(),
    expiresAt: z.string().optional(),
  })
  .strict();

export const assistantPendingCredentialEffectResponseSchema = z
  .object({
    resource: z.object({ pendingCredentialId: z.string().min(1) }).strict(),
    replayed: z.boolean(),
    requestedAt: z.string(),
  })
  .strict();

export const assistantDeviceEffectResponseSchema = z
  .object({
    resource: z.object({ deviceId: z.string().min(1) }).strict(),
    replayed: z.boolean(),
    requestedAt: z.string(),
    oneTimeMaterial: oneTimeMaterialSchema,
    qrPayload: z.string().min(1).optional(),
    expiresAt: z.string().optional(),
  })
  .strict();

export const nodeAuthorizationEvidenceSchema = z
  .object({
    id: z.string().min(1),
    owner_user_id: z.string().min(1),
    lifecycle: z.enum(["registration_pending", "active"]),
    is_active: z.boolean(),
    state_version: z.number().int().positive(),
    access_revision: z.number().int().nonnegative(),
    created_at: z.string(),
    updated_at: z.string(),
    registration_expires_at: z.string().nullable(),
  })
  .strict();

export const pendingCredentialAuthorizationEvidenceSchema = z
  .object({
    id: z.string().min(1),
    node_id: z.string().min(1),
    owner_user_id: z.string().min(1),
    remote_state: z.string().nullable(),
    is_active: z.boolean(),
    created_at: z.string(),
    expires_at: z.string(),
    consumed_at: z.string().nullable(),
    declined_at: z.string().nullable(),
  })
  .strict();

export const deviceAuthorizationEvidenceSchema = z
  .object({
    id: z.string().min(1),
    owner_user_id: z.string().min(1),
    used: z.boolean(),
    redeemed_node_id: z.string().nullable(),
    created_at: z.string(),
    expires_at: z.string(),
  })
  .strict();

export type NodeAuthorizationEvidence = z.infer<
  typeof nodeAuthorizationEvidenceSchema
>;

export async function readNodeAuthorization(nodeId: string) {
  const value = await api.get<unknown>(
    `/nodes/${encodeURIComponent(nodeId)}/authorization`,
  );
  assertSecretFreeReadBack(value);
  return nodeAuthorizationEvidenceSchema.parse(value);
}

export async function readPendingCredentialAuthorization(
  nodeId: string,
  pendingCredentialId: string,
) {
  const value = await api.get<unknown>(
    `/nodes/${encodeURIComponent(nodeId)}/credentials/pending/${encodeURIComponent(pendingCredentialId)}/authorization`,
  );
  assertSecretFreeReadBack(value);
  return pendingCredentialAuthorizationEvidenceSchema.parse(value);
}

export async function readDeviceAuthorization(deviceId: string) {
  const value = await api.get<unknown>(
    `/devices/onboard/${encodeURIComponent(deviceId)}/authorization`,
  );
  assertSecretFreeReadBack(value);
  return deviceAuthorizationEvidenceSchema.parse(value);
}

export function oneTimeMaterialUnavailable(
  status: "delivered" | "unavailable" | undefined,
  valuesPresent: boolean,
): boolean {
  // Older backends omit the marker. Treat that omission as "delivered", but
  // never imply the browser captured material that is absent from its response.
  return status === "unavailable" || !valuesPresent;
}
