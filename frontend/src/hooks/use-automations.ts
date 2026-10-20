import { z } from "zod";
import {
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  runsSchema,
  runSchema,
  type ScheduleSpec,
} from "@/schemas/automations";
export function useAutomationRuns(id: string | undefined) {
  return useInfiniteQuery({
    queryKey: ["triggers", id, "runs"],
    enabled: Boolean(id),
    initialPageParam: undefined as
      | { before: string; before_id: string }
      | undefined,
    queryFn: async ({ pageParam }) =>
      runsSchema.parse(
        await api.get(
          `/triggers/${encodeURIComponent(id!)}/runs${pageParam ? `?${new URLSearchParams(pageParam)}` : ""}`,
        ),
      ),
    getNextPageParam: (lastPage) => lastPage.next_cursor ?? undefined,
    refetchInterval: 5000,
  });
}
export function useRunAutomation() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: async (id: string) =>
      runSchema.parse(
        await api.post(`/triggers/${encodeURIComponent(id)}/run`),
      ),
    onSuccess: () => {
      void client.invalidateQueries({ queryKey: ["triggers"] });
    },
  });
}
export function useSchedulePreview(schedule: ScheduleSpec | undefined) {
  return useQuery({
    queryKey: ["triggers", "preview", schedule],
    enabled: Boolean(schedule),
    retry: false,
    staleTime: 30_000,
    queryFn: () =>
      api.post<{ next_runs: string[]; description?: string }>(
        "/triggers/preview",
        { schedule },
      ),
  });
}

export function useAutomationChats(enabled: boolean) {
  return useQuery({
    queryKey: ["triggers", "chats"],
    enabled,
    queryFn: async () => {
      const { nyxBotApi } = await import("@/lib/assistant/nyxbot-api");
      const channels = await nyxBotApi.channels();
      return (
        await Promise.all(
          channels.map((channel) => nyxBotApi.channelChats(channel.id)),
        )
      )
        .flat()
        .filter((chat) => chat.allow_posts);
    },
  });
}

const setupSchema = z.object({
  label: z.string(),
  instruction: z.string(),
  agent_id: z.string(),
  confirmation_policy: z.enum(["changes", "destructive"]),
  thread_policy: z.enum(["home", "dedicated", "new"]).nullish(),
});
export type AutomationSetup = z.infer<typeof setupSchema>;
export function useAutomationSetup(id: string | undefined) {
  return useQuery({
    queryKey: ["triggers", "setup", id],
    enabled: Boolean(id),
    retry: false,
    queryFn: async () =>
      setupSchema.parse(
        await api.get(`/triggers/setup/${encodeURIComponent(id!)}`),
      ),
  });
}
