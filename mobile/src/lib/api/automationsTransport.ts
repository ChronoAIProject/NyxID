import { z } from "zod";
import type { RequestOptions } from "./authenticatedTransport";
import {
  automationChatSchema,
  automationCreateRequestSchema,
  automationCreateResponseSchema,
  automationDeleteResponseSchema,
  automationDeviceConversationSchema,
  automationIdSchema,
  automationListSchema,
  automationPreferencesUpdateSchema,
  automationPreviewSchema,
  automationReferenceSchema,
  automationRunSchema,
  automationRunsPath,
  automationRunsSchema,
  automationSchema,
  automationSettingsSchema,
  automationSetupSchema,
  automationUpdateRequestSchema,
  automationUpdateResponseSchema,
  scheduleSchema,
  validateSchedule,
  type AutomationCreateRequest,
  type AutomationPreferencesUpdate,
  type AutomationRunsCursor,
  type AutomationUpdateRequest,
  type ScheduleSpec,
} from "../../features/automations/automationContracts";

const ROOT = "/triggers";
const ASSISTANT_ROOT = "/assistant/nyxagent";
const path = (id: string) => `${ROOT}/${automationIdSchema.parse(id)}`;
type JsonRequest = (path: string, options?: RequestOptions) => Promise<unknown>;
const previewRequestSchema = z
  .object({ schedule: scheduleSchema })
  .strict()
  .superRefine((value, context) => validateSchedule(value.schedule, context));

/** The injectable transport keeps API-boundary tests independent of Expo. */
export function createAutomationsApi(request: JsonRequest) {
  const assertTrigger = <T extends { trigger: { id: string } }>(
    id: string,
    result: T,
  ): T => {
    if (result.trigger.id !== id)
      throw new Error(
        "The server returned a different automation. Please reload.",
      );
    return result;
  };
  return {
    async list(signal?: AbortSignal, orgId?: string) {
      const suffix = orgId
        ? `?${new URLSearchParams({ org_id: automationIdSchema.parse(orgId) })}`
        : "";
      return automationListSchema.parse(
        await request(`${ROOT}${suffix}`, { signal }),
      );
    },
    async detail(id: string, signal?: AbortSignal) {
      const row = automationSchema.parse(await request(path(id), { signal }));
      if (row.id !== id)
        throw new Error(
          "The server returned a different automation. Please reload.",
        );
      return row;
    },
    async create(input: AutomationCreateRequest) {
      const body = automationCreateRequestSchema.parse(input);
      return automationCreateResponseSchema.parse(
        await request(ROOT, { method: "POST", body }),
      );
    },
    async update(id: string, input: AutomationUpdateRequest) {
      const body = automationUpdateRequestSchema.parse(input);
      return assertTrigger(
        id,
        automationUpdateResponseSchema.parse(
          await request(path(id), { method: "PATCH", body }),
        ),
      );
    },
    async remove(id: string) {
      return automationDeleteResponseSchema.parse(
        await request(path(id), { method: "DELETE" }),
      );
    },
    async run(id: string) {
      return automationRunSchema.parse(
        await request(`${path(id)}/run`, { method: "POST" }),
      );
    },
    async runs(
      id: string,
      cursor?: AutomationRunsCursor,
      signal?: AbortSignal,
    ) {
      return automationRunsSchema.parse(
        await request(automationRunsPath(id, cursor), { signal }),
      );
    },
    async preview(schedule: ScheduleSpec, signal?: AbortSignal) {
      const body = previewRequestSchema.parse({ schedule });
      return automationPreviewSchema.parse(
        await request(`${ROOT}/preview`, { method: "POST", body, signal }),
      );
    },
    async setup(id: string, signal?: AbortSignal) {
      return automationSetupSchema.parse(
        await request(`${ROOT}/setup/${automationIdSchema.parse(id)}`, {
          signal,
        }),
      );
    },
    async chats(signal?: AbortSignal) {
      const channels = z
        .object({
          channel_agents: z.array(z.object({ id: automationReferenceSchema })),
        })
        .parse(
          await request(`${ASSISTANT_ROOT}/channels`, { signal }),
        ).channel_agents;
      const lists = await Promise.all(
        channels.map(async (channel) => {
          const result = z
            .object({ chats: z.array(automationChatSchema) })
            .parse(
              await request(
                `${ASSISTANT_ROOT}/channels/${encodeURIComponent(channel.id)}/chats`,
                { signal },
              ),
            );
          if (result.chats.some((chat) => chat.channel_agent_id !== channel.id))
            throw new Error(
              "The server returned chats from a different channel. Please reload.",
            );
          return result.chats;
        }),
      );
      return lists.flat().filter((chat) => chat.allow_posts);
    },
    async settings(signal?: AbortSignal) {
      return automationSettingsSchema.parse(
        await request(`${ASSISTANT_ROOT}/settings`, { signal }),
      );
    },
    async deviceConversations(signal?: AbortSignal) {
      // Without org_id the existing endpoint returns only the acting person's
      // conversations. Device conversations have no NyxBot channel or bot.
      const result = z
        .object({
          conversations: z.array(automationDeviceConversationSchema),
          total: z.number().int().nonnegative(),
        })
        .parse(await request("/channel-conversations", { signal }));
      return result.conversations.filter(
        (conversation) =>
          conversation.platform === "device" && conversation.is_active,
      );
    },
    async updatePreferences(input: AutomationPreferencesUpdate) {
      const body = automationPreferencesUpdateSchema.parse(input);
      return automationSettingsSchema.parse(
        await request(`${ASSISTANT_ROOT}/settings`, { method: "PUT", body }),
      );
    },
    async rotateSecret(id: string) {
      return assertTrigger(
        id,
        z
          .object({ trigger: automationSchema, secret: z.string().min(1) })
          .parse(
            await request(`${path(id)}/rotate-secret`, { method: "POST" }),
          ),
      );
    },
  };
}
