import { z } from "zod";
import {
  REMOTE_MACHINE_PROFILE,
  REMOTE_MACHINE_POLICY,
  REMOTE_MACHINE_DATA_POLICY,
  remoteMachineNameSchema,
  remoteMachineCapabilitiesSchema,
} from "@/schemas/machines";

// Only safe owner lifecycle fields belong here. Never persist bootstrap material,
// command parameters, output, credentials, or OAuth approval tokens.
const version = z
  .string()
  .min(1)
  .max(128)
  .regex(/^[a-zA-Z0-9._-]+$/);
const bodySchema = z
  .object({
    free_slot: z.boolean().optional(),
    profile_version: z.literal(REMOTE_MACHINE_PROFILE).optional(),
    name: remoteMachineNameSchema.optional(),
    capabilities: remoteMachineCapabilitiesSchema.optional(),
    specialist_id: z.string().min(1).nullable().optional(),
    action: z.enum(["pause", "resume", "rekey", "delete"]).optional(),
    expected_revision: z.number().int().nonnegative().optional(),
    policy_version: z.literal(REMOTE_MACHINE_POLICY).optional(),
    data_policy_version: z.literal(REMOTE_MACHINE_DATA_POLICY).optional(),
    acknowledge_shared_environment: z.boolean().optional(),
    acknowledge_data_loss: z.boolean().optional(),
    // Public quote IDs and owner acceptance contain no bearer credentials.
    quote_id: version.optional(),
    accept_account_balance: z.boolean().optional(),
  })
  .strict()
  .superRefine((body, context) => {
    if (
      body.action &&
      (body.name !== undefined ||
        body.capabilities !== undefined ||
        body.specialist_id !== undefined)
    ) {
      context.addIssue({
        code: "custom",
        message: "Hosted configuration belongs only to creation requests",
      });
    }
    const paid =
      body.free_slot === false ||
      body.quote_id !== undefined ||
      body.accept_account_balance === true;
    if (
      paid &&
      (!body.quote_id ||
        body.accept_account_balance !== true ||
        body.free_slot === true ||
        (body.action !== undefined && body.action !== "resume"))
    ) {
      context.addIssue({
        code: "custom",
        path: ["accept_account_balance"],
        message:
          "Paid lifecycle intent requires a quote and explicit account-balance acceptance",
      });
    }
  });
export type RemoteMachineIntentBody = z.infer<typeof bodySchema>;
const intentSchema = z
  .object({
    idempotencyKey: z.string().uuid(),
    body: bodySchema,
  })
  .strict();
export type RemoteMachineIntent = z.infer<typeof intentSchema>;

function storageKey(ownerId: string, scope: string) {
  if (!ownerId || !scope)
    throw new Error("Sign in before managing a remote machine.");
  return `nyxid:remote-machine-intent:${encodeURIComponent(ownerId)}:${encodeURIComponent(scope)}`;
}

export function readRemoteMachineIntent(
  ownerId: string,
  scope: string,
): RemoteMachineIntent | null {
  const raw = sessionStorage.getItem(storageKey(ownerId, scope));
  if (!raw) return null;
  const parsed = intentSchema.safeParse(JSON.parse(raw));
  if (!parsed.success)
    throw new Error(
      "The saved machine request cannot be read. Review and reset the request before continuing.",
    );
  return parsed.data;
}

/** A reload or unknown network outcome retries the same reviewed body and key. */
export function getRemoteMachineIntent(
  ownerId: string,
  scope: string,
  body: RemoteMachineIntentBody,
): RemoteMachineIntent {
  const safeBody = bodySchema.parse(body);
  const existing = readRemoteMachineIntent(ownerId, scope);
  if (existing) return existing;
  const intent = { idempotencyKey: crypto.randomUUID(), body: safeBody };
  // Persist before dispatch. If browser storage is unavailable, fail closed so
  // refreshing cannot accidentally generate a second resource intent.
  sessionStorage.setItem(storageKey(ownerId, scope), JSON.stringify(intent));
  return intent;
}

export function clearRemoteMachineIntent(
  ownerId: string,
  scope: string,
  idempotencyKey?: string,
): void {
  if (
    idempotencyKey &&
    readRemoteMachineIntent(ownerId, scope)?.idempotencyKey !== idempotencyKey
  )
    return;
  sessionStorage.removeItem(storageKey(ownerId, scope));
}
