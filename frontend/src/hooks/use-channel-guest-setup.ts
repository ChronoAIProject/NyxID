import { useMutation, useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  guestPreviewSchema,
  guestSetupResponseSchema,
  type GuestSetupConfiguration,
} from "@/schemas/channel-guest-access";

import type { GuestSkillSelection } from "@/schemas/channel-guest-skills";

export const guestSetupQueryKey = (botId: string, staging = false) => ["channel-guest-setup", botId, staging];
const endpoint = (botId: string) => "/channel-bots/" + encodeURIComponent(botId) + "/guest-setup";

export function useChannelGuestSetup(botId: string, staging = false, enabled = true) {
  return useQuery({
    queryKey: guestSetupQueryKey(botId, staging),
    enabled,
    queryFn: async () => guestSetupResponseSchema.parse(await api.get(endpoint(botId) + (staging ? "/staging" : ""))),
    retry: false,
  });
}

export function usePreviewGuestSetup(botId: string, staging: boolean) {
  return useMutation({
    mutationFn: async (configuration: GuestSetupConfiguration) =>
      guestPreviewSchema.parse(await api.post(endpoint(botId) + "/preview", { configuration, ...(staging ? { staging } : {}) })),
  });
}

export function useSaveGuestSetup(botId: string, staging: boolean) {
  return useMutation({
    mutationFn: async ({
      apply, revision, configuration,
    }: {
      apply: boolean;
      revision: number;
      configuration: GuestSetupConfiguration;
    }) => {
      const body = { expected_revision: revision, configuration, ...(staging ? { staging } : {}) };
      const result = apply
        ? await api.post(endpoint(botId) + "/apply", body)
        : await api.put(endpoint(botId) + "/draft", body);
      return guestSetupResponseSchema.parse(result);
    },
  });
}

export function useGuestSetupPreference(botId: string) {
  return useMutation({
    mutationFn: async ({ enabled, revision }: { enabled: boolean; revision: number }) =>
      guestSetupResponseSchema.parse(await api.put(endpoint(botId), { enabled, expected_bot_revision: revision })),
  });
}

export function usePrepareGuestSetup(botId: string) {
  return useMutation({
    mutationFn: async ({ revision, skillSelection }: { revision: number; skillSelection?: GuestSkillSelection }) =>
      guestSetupResponseSchema.parse(await api.post(endpoint(botId) + "/prepare", {
        expected_revision: revision,
        skill_selection: skillSelection,
      })),
  });
}
