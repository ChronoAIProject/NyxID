import { useCallback, useRef } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { nyxBotApi } from "@/lib/assistant/nyxbot-api";
import { nyxBotQueryKeys } from "@/hooks/use-nyxbot-agents";
import type {
  AssistantGroup,
  AssistantGroupForm,
  AssistantGroupMessage,
  AssistantGroupMessages,
  AssistantGroupUpdate,
} from "@/schemas/assistant-nyxagent";
import { useAuthStore } from "@/stores/auth-store";

export const nyxBotGroupKeys = {
  /** Prefix of the group list, every group and every transcript. */
  all: (userId: string | undefined) => ["assistant", "nyxagent", userId, "groups"] as const,
  messages: (userId: string | undefined, groupId: string | undefined) =>
    ["assistant", "nyxagent", userId, "groups", "messages", groupId] as const,
};

/** While an agent is replying, the transcript is re-read quickly. */
export const GROUP_WORKING_POLL_MS = 1500;
/** An open, quiet group still picks up replies that arrive later. */
export const GROUP_IDLE_POLL_MS = 15_000;
/** After posting, poll quickly for a while even before anyone is marked working. */
export const GROUP_AFTER_POST_MS = 5000;
const GROUPS_WORKING_POLL_MS = 3000;

export function groupPollInterval(
  page: AssistantGroupMessages | undefined,
  fastUntil: number,
  now = Date.now(),
): number {
  return (page?.group.working_agent_ids.length ?? 0) > 0 || now < fastUntil
    ? GROUP_WORKING_POLL_MS
    : GROUP_IDLE_POLL_MS;
}

/** Newest activity first. Re-read while any group has an agent working. */
export function useNyxBotGroups(enabled = true) {
  const userId = useAuthStore((state) => state.user?.id);
  return useQuery({
    queryKey: [...nyxBotGroupKeys.all(userId), "list"],
    queryFn: () => nyxBotApi.groups(),
    enabled: enabled && Boolean(userId),
    retry: false,
    refetchInterval: (query) =>
      query.state.data?.some((group) => group.working_agent_ids.length > 0)
        ? GROUPS_WORKING_POLL_MS
        : false,
  });
}

function mergeMessages(
  older: readonly AssistantGroupMessage[],
  newer: readonly AssistantGroupMessage[],
): AssistantGroupMessage[] {
  const bySeq = new Map(older.map((message) => [message.seq, message]));
  for (const message of newer) bySeq.set(message.seq, message);
  return [...bySeq.values()].sort((a, b) => a.seq - b.seq);
}

/** Keep the list row in step with the transcript's view of the group. */
function patchGroupList(
  groups: AssistantGroup[] | undefined,
  group: AssistantGroup,
): AssistantGroup[] | undefined {
  return groups?.map((row) => (row.id === group.id ? group : row));
}

/**
 * One group's transcript. Polling follows the contract: every 1.5 s while an
 * agent is working (and for a few seconds after posting), else every 15 s.
 * Older pages are merged into the cached transcript by `seq`.
 */
export function useNyxBotGroupMessages(groupId: string | undefined) {
  const userId = useAuthStore((state) => state.user?.id);
  const queryClient = useQueryClient();
  const key = nyxBotGroupKeys.messages(userId, groupId);
  const fastUntil = useRef(0);
  const listKey = [...nyxBotGroupKeys.all(userId), "list"];
  const query = useQuery({
    queryKey: key,
    queryFn: async () => {
      const previous = queryClient.getQueryData<AssistantGroupMessages>(key);
      const page = await nyxBotApi.groupMessages(groupId!);
      queryClient.setQueryData<AssistantGroup[]>(listKey, (groups) =>
        patchGroupList(groups, page.group),
      );
      if (!previous) return page;
      // Keep older pages that were loaded; the newest page replaces its range.
      const oldest = page.messages[0]?.seq ?? Number.POSITIVE_INFINITY;
      const kept = previous.messages.filter((message) => message.seq < oldest);
      return {
        ...page,
        messages: mergeMessages(kept, page.messages),
        before_seq: kept.length ? previous.before_seq : page.before_seq,
      };
    },
    enabled: Boolean(userId && groupId),
    retry: false,
    refetchInterval: (state) => groupPollInterval(state.state.data, fastUntil.current),
  });

  const loadOlder = useCallback(async () => {
    const current = queryClient.getQueryData<AssistantGroupMessages>(key);
    if (!groupId || !current?.before_seq) return;
    const page = await nyxBotApi.groupMessages(groupId, current.before_seq);
    queryClient.setQueryData<AssistantGroupMessages>(key, (latest) =>
      latest
        ? {
            ...latest,
            messages: mergeMessages(page.messages, latest.messages),
            before_seq: page.before_seq,
          }
        : latest,
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [groupId, queryClient, userId]);

  const post = useMutation({
    mutationFn: (text: string) => nyxBotApi.postGroupMessage(groupId!, text),
    onSuccess: (posted) => {
      fastUntil.current = Date.now() + GROUP_AFTER_POST_MS;
      // Show the message and who it went to straight away; the next poll
      // replaces this with the server's view.
      queryClient.setQueryData<AssistantGroupMessages>(key, (latest) => {
        if (!latest) return latest;
        const working = [
          ...new Set([...latest.group.working_agent_ids, ...posted.addressed_agent_ids]),
        ];
        return {
          ...latest,
          group: {
            ...latest.group,
            working_agent_ids: working,
            members: latest.group.members.map((member) => ({
              ...member,
              working: working.includes(member.id),
            })),
          },
          messages: mergeMessages(latest.messages, [posted.message]),
        };
      });
    },
    onSettled: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: key }),
        // Addressed members start working through their own threads.
        queryClient.invalidateQueries({ queryKey: nyxBotQueryKeys.agents(userId) }),
      ]);
    },
  });

  return { ...query, loadOlder, post };
}

function useGroupsMutation<TVariables, TResult>(
  mutationFn: (variables: TVariables) => Promise<TResult>,
) {
  const userId = useAuthStore((state) => state.user?.id);
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn,
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: nyxBotGroupKeys.all(userId) });
    },
  });
}

export function useCreateNyxBotGroup() {
  return useGroupsMutation((body: AssistantGroupForm) => nyxBotApi.createGroup(body));
}

export function useUpdateNyxBotGroup() {
  return useGroupsMutation(({ id, ...body }: AssistantGroupUpdate & { id: string }) =>
    nyxBotApi.updateGroup(id, body),
  );
}

export function useDeleteNyxBotGroup() {
  const userId = useAuthStore((state) => state.user?.id);
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => nyxBotApi.deleteGroup(id),
    onSuccess: (_, id) => {
      queryClient.removeQueries({ queryKey: nyxBotGroupKeys.messages(userId, id) });
    },
    onSettled: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: [...nyxBotGroupKeys.all(userId), "list"] }),
        // Agents may have stopped working for the deleted group.
        queryClient.invalidateQueries({ queryKey: nyxBotQueryKeys.agents(userId) }),
      ]);
    },
  });
}
