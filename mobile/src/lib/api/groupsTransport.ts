import { z } from "zod";
import type { RequestOptions } from "./authenticatedTransport";
import {
  acknowledgementSchema,
  conversationIdSchema,
} from "../../features/assistant/assistantTypes";
import {
  groupCreateSchema,
  groupFeedbackInputSchema,
  groupFeedbackSchema,
  groupIdSchema,
  groupIdentitySchema,
  groupListSchema,
  groupMessagesPageSchema,
  groupOrganizationMemberSchema,
  groupOrganizationSchema,
  groupPendingActionSchema,
  groupPostedSchema,
  groupPostRequestSchema,
  groupSchema,
  groupUpdateSchema,
  type GroupCreate,
  type GroupFeedbackInput,
  type GroupPendingAction,
  type GroupPostRequest,
  type GroupUpdate,
} from "../../features/groups/groupContracts";

const ROOT = "/assistant/nyxagent/groups";
const path = (id: string) => `${ROOT}/${groupIdSchema.parse(id)}`;
type JsonRequest = (path: string, options?: RequestOptions) => Promise<unknown>;
const approvedResponse = z.object({
  id: z.string().uuid(),
  status: z.enum(["approved", "rejected"]),
  decided_at: z.string().datetime({ offset: true }).nullable(),
});

/** No native imports: tests can exercise the exact HTTP boundary. */
export function createGroupsApi(request: JsonRequest) {
  const sameGroup = <T extends { id: string }>(id: string, group: T): T => {
    if (group.id !== id)
      throw new Error("The server returned a different group. Please reload.");
    return group;
  };
  async function organizations(signal?: AbortSignal) {
    return z
      .object({ orgs: z.array(groupOrganizationSchema) })
      .parse(await request("/orgs", { signal })).orgs;
  }
  async function organizationMembers(orgId: string, signal?: AbortSignal) {
    const id = z.string().uuid().parse(orgId);
    return z
      .object({ members: z.array(groupOrganizationMemberSchema) })
      .parse(await request(`/orgs/${id}/members`, { signal })).members;
  }
  const feedbackPath = (id: string, messageId: string) =>
    `${path(id)}/messages/${z.string().uuid().parse(messageId)}/feedback`;
  async function feedback(
    id: string,
    messageId: string,
    input: GroupFeedbackInput,
  ) {
    const body = groupFeedbackInputSchema.parse(input);
    return groupFeedbackSchema.parse(
      await request(feedbackPath(id, messageId), { method: "PUT", body }),
    );
  }
  async function removeFeedback(id: string, messageId: string): Promise<null> {
    await request(feedbackPath(id, messageId), { method: "DELETE" });
    return null;
  }
  return {
    async list(signal?: AbortSignal) {
      return groupListSchema.parse(await request(ROOT, { signal })).groups;
    },
    async detail(id: string, signal?: AbortSignal) {
      return sameGroup(
        id,
        groupSchema.parse(await request(path(id), { signal })),
      );
    },
    async create(input: GroupCreate) {
      const body = groupCreateSchema.parse(input);
      return groupSchema.parse(await request(ROOT, { method: "POST", body }));
    },
    async update(id: string, input: GroupUpdate) {
      const body = groupUpdateSchema.parse(input);
      return sameGroup(
        id,
        groupSchema.parse(await request(path(id), { method: "PATCH", body })),
      );
    },
    async leave(id: string) {
      return sameGroup(
        id,
        groupSchema.parse(
          await request(path(id), { method: "PATCH", body: { leave: true } }),
        ),
      );
    },
    async remove(id: string) {
      await request(path(id), { method: "DELETE" });
    },
    async messages(id: string, beforeSeq?: number, signal?: AbortSignal) {
      const params = new URLSearchParams({ limit: "50" });
      if (beforeSeq !== undefined)
        params.set(
          "before_seq",
          String(z.number().int().positive().parse(beforeSeq)),
        );
      const page = groupMessagesPageSchema.parse(
        await request(`${path(id)}/messages?${params}`, { signal }),
      );
      sameGroup(id, page.group);
      return page;
    },
    async post(id: string, input: GroupPostRequest) {
      const body = groupPostRequestSchema.parse(input);
      return groupPostedSchema.parse(
        await request(`${path(id)}/messages`, { method: "POST", body }),
      );
    },
    async identity(signal?: AbortSignal) {
      return groupIdentitySchema.parse(await request("/users/me", { signal }));
    },
    async features(signal?: AbortSignal) {
      return groupIdentitySchema.parse(await request("/users/me", { signal }))
        .enabled_features;
    },
    organizations,
    orgs: organizations,
    organizationMembers,
    orgMembers: organizationMembers,
    feedback,
    putFeedback: feedback,
    setFeedback: feedback,
    removeFeedback,
    deleteFeedback: removeFeedback,
    async decide(input: GroupPendingAction, decision: "allow" | "deny") {
      const action = groupPendingActionSchema.parse(input);
      z.enum(["allow", "deny"]).parse(decision);
      if (action.authored_skill)
        throw new Error("Review this draft in its conversation.");
      if (action.approval_request_id) {
        const result = approvedResponse.parse(
          await request(
            `/approvals/requests/${action.approval_request_id}/decide`,
            { method: "POST", body: { approved: decision === "allow" } },
          ),
        );
        if (result.id !== action.approval_request_id)
          throw new Error("Approval request mismatch.");
        return result;
      }
      if (!action.acknowledgement_id)
        throw new Error("This action has no decision reference.");
      const conversation = conversationIdSchema.parse(action.conversation_id);
      const result = acknowledgementSchema.parse(
        await request(
          `/assistant/nyxagent/conversations/${conversation}/acknowledgements/${action.acknowledgement_id}`,
          { method: "POST", body: { decision } },
        ),
      );
      if (result.id !== action.acknowledgement_id)
        throw new Error("Assistant acknowledgement mismatch.");
      return result;
    },
  };
}
