import { z } from "zod";
import type { RequestOptions } from "./authenticatedTransport";
import {
  groupSchema,
  groupIdSchema,
} from "../../features/groups/groupContracts";
import {
  groupInviteSchema,
  groupInvitePreviewSchema,
  type CollaborationChange,
} from "../../features/groups/collaborationContracts";
import {
  sharingSchema,
  sharingFormSchema,
  sharedNotesSchema,
  sharedConsentsSchema,
  type SharingForm,
} from "../../features/agents/sharingContracts";
import {
  agentServiceOperationsSchema,
  operationSelectionSchema,
  type OperationSelection,
} from "../../features/agents/operationContracts";

type Request = (path: string, options?: RequestOptions) => Promise<unknown>;
type MutationOptions = Pick<RequestOptions, "authorityGuard">;
const ROOT = "/assistant/nyxagent";
const agentPath = (id: string) => `${ROOT}/agents/${encodeURIComponent(id)}`;
const groupPath = (id: string) => `${ROOT}/groups/${groupIdSchema.parse(id)}`;
const revision = z.number().int().nonnegative();
const notesUpdate = z.object({
  expected_revision: revision,
  notes: z
    .array(
      z
        .string()
        .refine(
          (value) => [...value].length <= 500,
          "A note has at most 500 characters.",
        ),
    )
    .max(50),
});

/** Exact human REST contracts; no fallback to legacy acknowledgements for consent. */
export function createCollaborationApi(request: Request) {
  return {
    async available(signal?: AbortSignal) {
      return z
        .object({ available: z.boolean() })
        .parse(await request(`${ROOT}/collaboration`, { signal })).available;
    },
    async change(
      id: string,
      expected_revision: number,
      change: CollaborationChange,
      options?: MutationOptions,
    ) {
      const result = z
        .object({
          group: groupSchema.nullable(),
          invite: groupInviteSchema.nullable(),
        })
        .parse(
          await request(`${groupPath(id)}/membership`, {
            ...options,
            method: "POST",
            body: {
              expected_revision: revision.min(1).parse(expected_revision),
              ...change,
            },
          }),
        );
      if (result.group && result.group.id !== id)
        throw new Error("Group response mismatch. Reload before continuing.");
      return result;
    },
    async preview(token: string, signal?: AbortSignal) {
      return groupInvitePreviewSchema.parse(
        await request(`${ROOT}/group-invites/preview`, {
          method: "POST",
          body: { token: inviteToken(token) },
          signal,
        }),
      );
    },
    async accept(token: string, options?: MutationOptions) {
      return groupSchema.parse(
        await request(`${ROOT}/group-invites/accept`, {
          ...options,
          method: "POST",
          body: { token: inviteToken(token), history_disclosed: true },
        }),
      );
    },
    async decide(
      group: string,
      id: string,
      kind: "consent" | "action" | "approval",
      decision: string,
      options?: MutationOptions,
    ) {
      const allowed =
        kind === "consent" ? ["once", "always", "skip"] : ["allow", "deny"];
      if (!allowed.includes(decision))
        throw new Error("Invalid decision for this card.");
      const target =
        kind === "approval"
          ? `/approvals/requests/${encodeURIComponent(id)}/decide`
          : `${groupPath(group)}/decisions/${encodeURIComponent(id)}`;
      await request(target, {
        ...options,
        method: "POST",
        body:
          kind === "approval"
            ? { approved: decision === "allow" }
            : { kind, decision },
      });
    },
    async cancel(group: string, id: string, options?: MutationOptions) {
      await request(
        `${groupPath(group)}/requests/${encodeURIComponent(id)}/cancel`,
        { ...options, method: "POST" },
      );
    },
    async sharing(id: string, signal?: AbortSignal) {
      return sharingSchema.parse(
        await request(`${agentPath(id)}/sharing`, { signal }),
      );
    },
    async saveSharing(
      id: string,
      version: number,
      input: SharingForm,
      options?: MutationOptions,
    ) {
      await request(`${agentPath(id)}/sharing`, {
        ...options,
        method: "PUT",
        body: {
          ...sharingFormSchema.parse(input),
          expected_version: revision.parse(version),
        },
      });
    },
    async notes(
      id: string,
      kind: "private-notes" | "shared-memory",
      signal?: AbortSignal,
    ) {
      return sharedNotesSchema.parse(
        await request(`${agentPath(id)}/${kind}`, { signal }),
      );
    },
    async saveNotes(
      id: string,
      kind: "private-notes" | "shared-memory",
      expected_revision: number,
      notes: string[],
      options?: MutationOptions,
    ) {
      await request(`${agentPath(id)}/${kind}`, {
        ...options,
        method: "PUT",
        body: notesUpdate.parse({ expected_revision, notes }),
      });
    },
    async consents(id: string, signal?: AbortSignal) {
      return sharedConsentsSchema.parse(
        await request(`${agentPath(id)}/consents`, { signal }),
      );
    },
    async decideConsent(
      id: string,
      decision: "once" | "always" | "skip" | "revoked",
      options?: MutationOptions,
    ) {
      await request(`${ROOT}/shared-consents/${encodeURIComponent(id)}`, {
        ...options,
        method: "POST",
        body: { decision },
      });
    },
    async operations(id: string, guests: boolean, signal?: AbortSignal) {
      return agentServiceOperationsSchema
        .array()
        .parse(
          await request(
            `${agentPath(id)}/${guests ? "guest-operations" : "operations"}`,
            { signal },
          ),
        );
    },
    async saveOperations(
      id: string,
      guests: boolean,
      serviceId: string,
      selection: OperationSelection,
      options?: MutationOptions,
    ) {
      await request(
        `${agentPath(id)}/${guests ? "guest-operations" : "operations"}/${encodeURIComponent(serviceId)}`,
        {
          ...options,
          method: "PUT",
          body: operationSelectionSchema.parse(selection),
        },
      );
    },
    async readiness(id: string, signal?: AbortSignal) {
      return z
        .object({
          status: z.enum(["ready", "setting_up", "not_ready"]),
          fix: z.string().nullish(),
          reason: z.string().nullish(),
        })
        .parse(await request(`${agentPath(id)}/guest-readiness`, { signal }));
    },
  };
}

/** Paste either the Web invitation link or its one-use token. Never follow a URL. */
export function inviteToken(input: string) {
  const value = input.trim();
  const token = value.includes("#")
    ? new URLSearchParams(value.slice(value.indexOf("#") + 1)).get(
        "group-invite",
      )
    : value;
  const parsed = z
    .string()
    .length(64)
    .regex(/^[a-fA-F0-9]+$/)
    .safeParse(token);
  if (!parsed.success)
    throw new Error(
      "Paste the full invitation link or a 64-character invitation code.",
    );
  return parsed.data;
}

/** The frontend origin is build-configured; pasted invitations never select it. */
export function invitationLink(
  frontend: string,
  token: string,
): string | undefined {
  try {
    const base = new URL(frontend);
    if (
      !["http:", "https:"].includes(base.protocol) ||
      base.username ||
      base.password
    )
      return undefined;
    const url = new URL("/assistant", base.origin);
    url.hash = `group-invite=${inviteToken(token)}`;
    return url.toString();
  } catch {
    return undefined;
  }
}

export function operationPayload(
  service: z.infer<typeof agentServiceOperationsSchema>,
  selection: OperationSelection,
): OperationSelection {
  if (selection.all_operations)
    return { ...selection, endpoint_ids: [], rules: [] };
  const inputs = Object.fromEntries(
    Object.entries(service.inputs ?? {}).filter(([id]) =>
      selection.endpoint_ids.includes(id),
    ),
  );
  return { ...selection, ...(Object.keys(inputs).length ? { inputs } : {}) };
}
