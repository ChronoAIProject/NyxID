import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { guestSkillInventorySchema } from "@/schemas/channel-guest-skills";

export const guestSkillsPath = (botId: string) =>
  `/channel-bots/${encodeURIComponent(botId)}/guest-skills`;
export const guestSkillsQueryKey = (botId: string) =>
  ["channel-guest-skills", botId] as const;

export function useGuestSkills(botId: string) {
  return useQuery({
    queryKey: guestSkillsQueryKey(botId),
    queryFn: async () =>
      guestSkillInventorySchema.parse(await api.get(guestSkillsPath(botId))),
    enabled: Boolean(botId),
  });
}
