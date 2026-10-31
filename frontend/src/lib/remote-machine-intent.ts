import { z } from "zod";
import {
  REMOTE_MACHINE_PROFILE,
  REMOTE_MACHINE_POLICY,
  REMOTE_MACHINE_DATA_POLICY,
  remoteMachineNameSchema,
  remoteMachineCapabilitiesSchema,
  remoteMachineCreateRequestSchema,
  machineSeparationChoiceSchema,
  REMOTE_MACHINE_SEPARATION_POLICY,
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
    separation_choice: machineSeparationChoiceSchema.optional(),
    separation_policy_version: z.literal(REMOTE_MACHINE_SEPARATION_POLICY).optional(),
    acknowledge_separation: z.boolean().optional(),
    action: z.enum(["pause", "resume", "rekey", "delete"]).optional(),
    expected_revision: z.number().int().nonnegative().optional(),
    expected_retry_count: z.number().int().nonnegative().optional(),
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
        body.specialist_id !== undefined ||
        body.separation_choice !== undefined ||
        body.separation_policy_version !== undefined ||
        body.acknowledge_separation !== undefined)
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
  })
  .transform((body, context) => {
    if (body.free_slot === undefined) return body;
    const create = remoteMachineCreateRequestSchema.safeParse(body);
    if (!create.success) {
      context.addIssue({ code: "custom", message: "A complete reviewed hosted creation request is required" });
      return z.NEVER;
    }
    return { ...body, ...create.data };
  });
export type RemoteMachineIntentBody = z.infer<typeof bodySchema>;
const intentSchema = z
  .object({
    idempotencyKey: z.string().uuid(),
    body: bodySchema,
  })
  .strict();
export type RemoteMachineIntent = z.infer<typeof intentSchema>;

const acceptedIntentSchema = intentSchema.extend({
  operationId: z.string().min(1),
  action: z.enum(["pause", "resume", "delete"]),
  observedRevision: z.number().int().nonnegative().optional(),
  currentObservation: z.object({
    operationId: z.string().min(1).nullable(),
    revision: z.number().int().nonnegative(),
  }).strict().optional(),
}).strict();
export type AcceptedRemoteMachineIntent = z.infer<typeof acceptedIntentSchema>;
export const REMOTE_MACHINE_ACCEPTED_EVENT = "nyxid:remote-machine-accepted";

function acceptedStorageKey(ownerId: string, machineId: string) {
  return `${storageKey(ownerId, machineId)}:accepted`;
}

export function remoteMachineAcceptedSnapshot(ownerId: string, machineId: string) {
  return sessionStorage.getItem(acceptedStorageKey(ownerId, machineId));
}

export function parseAcceptedRemoteMachineIntent(raw: string | null) {
  if (!raw) return null;
  const accepted = acceptedIntentSchema.parse(JSON.parse(raw));
  if (accepted.body.action !== accepted.action)
    throw new Error("The accepted machine request does not match its action.");
  return accepted;
}

/** Transfer the exact reviewed request to a response receipt before clearing it. */
export function acceptRemoteMachineIntent(
  ownerId: string,
  machineId: string,
  action: AcceptedRemoteMachineIntent["action"],
  idempotencyKey: string,
  operationId: string,
) {
  const scope = `${machineId}:${action}`;
  const intent = readRemoteMachineIntent(ownerId, scope);
  if (!intent || intent.idempotencyKey !== idempotencyKey) return;
  const accepted = acceptedIntentSchema.parse({ ...intent, action, operationId });
  if (accepted.body.action !== action)
    throw new Error("The accepted machine request does not match its action.");
  sessionStorage.setItem(acceptedStorageKey(ownerId, machineId), JSON.stringify(accepted));
  clearRemoteMachineIntent(ownerId, scope, idempotencyKey);
  window.dispatchEvent(new Event(REMOTE_MACHINE_ACCEPTED_EVENT));
}

/** Retain the highest real current observation independently of the request. */
export function observeAcceptedRemoteMachineIntent(
  ownerId: string,
  machineId: string,
  operationId: string | null,
  revision: number,
) {
  const accepted = parseAcceptedRemoteMachineIntent(remoteMachineAcceptedSnapshot(ownerId, machineId));
  if (!accepted) return;
  const highest = accepted.currentObservation?.revision ?? accepted.observedRevision ?? accepted.body.expected_revision ?? -1;
  if (revision <= highest) return;
  sessionStorage.setItem(acceptedStorageKey(ownerId, machineId), JSON.stringify({
    ...accepted,
    ...(operationId === accepted.operationId ? { observedRevision: revision } : {}),
    currentObservation: { operationId, revision },
  }));
  window.dispatchEvent(new Event(REMOTE_MACHINE_ACCEPTED_EVENT));
}

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
  if (!parsed.success || (scope === "create" && parsed.data.body.free_slot === undefined))
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
  if (scope === "create" && safeBody.free_slot === undefined)
    throw new Error("A complete reviewed hosted creation request is required before dispatch.");
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
