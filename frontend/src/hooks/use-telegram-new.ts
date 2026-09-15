import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import {
  telegramNewBeginSchema,
  telegramNewConfigSchema,
  telegramNewLaunchSchema,
  telegramNewRequestSchema,
  type TelegramNewRequest,
} from "@/schemas/telegram-new";

const ROOT = "/channel-bots/telegram-new";

export function useTelegramNewConfiguration() {
  const actor = useAuthStore((state) => state.user?.id);
  return useQuery({
    queryKey: ["telegram-new", actor],
    enabled: Boolean(actor),
    retry: false,
    staleTime: 0,
    refetchInterval: (query) => (query.state.data?.request ? 3000 : false),
    refetchIntervalInBackground: false,
    refetchOnWindowFocus: true,
    queryFn: async () => telegramNewConfigSchema.parse(await api.get(ROOT)),
  });
}

export function useTelegramNew() {
  const actor = useAuthStore((state) => state.user?.id);
  const client = useQueryClient();
  const key = ["telegram-new", actor] as const;
  const configuration = useTelegramNewConfiguration();
  const refresh = () => client.invalidateQueries({ queryKey: key });
  const begin = useMutation({
    gcTime: 0,
    mutationFn: async (input: { label: string; target_org_id?: string }) =>
      telegramNewLaunchSchema.parse(
        await api.post(ROOT, telegramNewBeginSchema.parse(input)),
      ),
    onSettled: refresh,
  });
  const launch = useMutation({
    gcTime: 0,
    mutationFn: async (id: string) =>
      telegramNewLaunchSchema.parse(
        await api.post(`${ROOT}/requests/${id}/launch`),
      ),
    onSettled: refresh,
  });
  const cancel = useMutation({
    mutationFn: (id: string) => api.delete(`${ROOT}/requests/${id}`),
    onSettled: refresh,
  });
  const connect = useMutation({
    mutationFn: async (request: TelegramNewRequest) =>
      telegramNewRequestSchema.parse(
        await api.post(`${ROOT}/requests/${request.id}/connect`, {
          telegram_bot_id: request.telegram_bot_id,
          revision: request.revision,
        }),
      ),
    onSettled: async () => {
      await refresh();
      await client.invalidateQueries({ queryKey: ["channel-bots"] });
    },
  });
  return { configuration, begin, launch, cancel, connect };
}
