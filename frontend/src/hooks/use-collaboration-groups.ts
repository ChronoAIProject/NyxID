import { useEffect, useRef } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { z } from "zod";
import { assistantJson } from "@/lib/assistant/assistant-http";
import { assistantGroupSchema } from "@/schemas/assistant-nyxagent";
import {
  groupInviteSchema,
  groupInvitePreviewSchema,
  guestAccessRequestSchema,
  type CollaborationChange,
} from "@/schemas/collaboration-group";
import { nyxBotGroupKeys } from "./use-nyxbot-groups";
import { useAuthStore } from "@/stores/auth-store";
import { useFeature } from "./use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";

const root = "/assistant/nyxagent";
const path = (id: string) => `${root}/groups/${encodeURIComponent(id)}`;
const mutationSchema = z.object({
  group: assistantGroupSchema.nullable(),
  invite: groupInviteSchema.nullable(),
});
export const collaborationApi = {
  async markRead(group: string, lastSeenSeq: number) {
    await assistantJson(`${path(group)}/read`, { method: "POST", body: { last_seen_seq: lastSeenSeq } });
  },
  async askOwner(group: string, agent: string) {
    return guestAccessRequestSchema.parse(
      await assistantJson(
        `${path(group)}/agents/${encodeURIComponent(agent)}/guest-requests`,
        { method: "POST" },
      ),
    );
  },
  async cancelGuestRequest(group: string, agent: string, request: string) {
    await assistantJson(
      `${path(group)}/agents/${encodeURIComponent(agent)}/guest-requests/${encodeURIComponent(request)}`,
      { method: "DELETE" },
    );
  },
  async change(id: string, revision: number, change: CollaborationChange) {
    return mutationSchema.parse(
      await assistantJson(`${path(id)}/membership`, {
        method: "POST",
        body: { expected_revision: revision, ...change },
      }),
    );
  },
  async preview(token: string) {
    return groupInvitePreviewSchema.parse(
      await assistantJson(`${root}/group-invites/preview`, {
        method: "POST",
        body: { token },
      }),
    );
  },
  async accept(token: string) {
    return assistantGroupSchema.parse(
      await assistantJson(`${root}/group-invites/accept`, {
        method: "POST",
        body: { token, history_disclosed: true },
      }),
    );
  },
  async decide(group: string, id: string, kind: string, decision: string) {
    if (kind === "approval") {
      await assistantJson(
        `/approvals/requests/${encodeURIComponent(id)}/decide`,
        { method: "POST", body: { approved: decision === "allow" } },
      );
    } else
      await assistantJson(
        `${path(group)}/decisions/${encodeURIComponent(id)}`,
        { method: "POST", body: { kind, decision } },
      );
  },
  async cancel(group: string, request: string) {
    await assistantJson(
      `${path(group)}/requests/${encodeURIComponent(request)}/cancel`,
      { method: "POST" },
    );
  },
};
export function useCollaborationGuestRequest(group: string) {
  const user = useAuthStore((s) => s.user?.id);
  const client = useQueryClient();
  return useMutation({
    mutationFn: async (input: { agent: string; cancelRequest?: string }) => {
      if (input.cancelRequest) {
        await collaborationApi.cancelGuestRequest(
          group,
          input.agent,
          input.cancelRequest,
        );
      } else {
        await collaborationApi.askOwner(group, input.agent);
      }
    },
    onSettled: () =>
      client.invalidateQueries({ queryKey: nyxBotGroupKeys.all(user) }),
  });
}
export function useCollaborationAvailable() {
  const user = useAuthStore((s) => s.user?.id);
  const enabled = useFeature(FEATURE_FLAG.SHARED_AGENTS);
  const query = useQuery({
    queryKey: ["assistant", user, "collaboration-available"],
    enabled: enabled && Boolean(user),
    retry: false,
    queryFn: async () =>
      z
        .object({ available: z.boolean() })
        .parse(await assistantJson(`${root}/collaboration`)).available,
  });
  return { ...query, data: enabled && user ? query.data : false };
}
export function useCollaborationChange(group: string, revision: number) {
  const user = useAuthStore((s) => s.user?.id);
  const client = useQueryClient();
  return useMutation({
    mutationFn: (change: CollaborationChange) =>
      collaborationApi.change(group, revision, change),
    onSettled: () =>
      client.invalidateQueries({ queryKey: nyxBotGroupKeys.all(user) }),
  });
}
export function useCollaborationDecision(group: string) {
  const user = useAuthStore((s) => s.user?.id);
  const client = useQueryClient();
  return useMutation({
    mutationFn: (input: { id: string; kind: string; decision: string }) =>
      collaborationApi.decide(group, input.id, input.kind, input.decision),
    onSettled: () =>
      client.invalidateQueries({ queryKey: nyxBotGroupKeys.all(user) }),
  });
}


/** Reading is explicit: background polling and pagination do not mark unseen messages. */
export function useCollaborationRead(group: string, seq: number, enabled: boolean) {
  const user = useAuthStore((s) => s.user?.id);
  const client = useQueryClient();
  const sharingEnabled = useFeature(FEATURE_FLAG.SHARED_AGENTS);
  const sent = useRef({ key: "", seq: 0 });
  useEffect(() => {
    const key = `${user}:${group}`;
    let disposed = false;
    const read = () => {
      if (!enabled || !sharingEnabled || !user || seq <= 0 || document.visibilityState === "hidden") return;
      if (sent.current.key === key && sent.current.seq >= seq) return;
      sent.current = { key, seq };
      void collaborationApi.markRead(group, seq).then(() => {
        if (!disposed) void client.invalidateQueries({ queryKey: [...nyxBotGroupKeys.all(user), "list"] });
      }).catch(() => {
        if (sent.current.key === key && sent.current.seq === seq) sent.current = { key, seq: 0 };
      });
    };
    read();
    document.addEventListener("visibilitychange", read);
    return () => { disposed = true; document.removeEventListener("visibilitychange", read); };
  }, [client, enabled, group, seq, sharingEnabled, user]);
}
