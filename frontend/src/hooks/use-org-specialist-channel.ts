import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { assistantJson } from "@/lib/assistant/assistant-http";
import { useAuthStore } from "@/stores/auth-store";
import {
  orgChannelSettingsSchema,
  orgChannelChallengeSchema,
  type OrgChannelGuestForm,
} from "@/schemas/org-specialist-channel";

export function useOrgSpecialistChannel(id: string, challenge?: string) {
  const userId = useAuthStore((s) => s.user?.id);
  const queryClient = useQueryClient();
  const root = `/assistant/nyxagent/channels/${encodeURIComponent(id)}`;
  const key = ["org-specialist-channel", userId, id];
  const settings = useQuery({
    queryKey: key,
    queryFn: async () =>
      orgChannelSettingsSchema.parse(
        await assistantJson(`${root}/org-specialist`),
      ),
    enabled: Boolean(userId && id),
    retry: false,
  });
  const save = useMutation({
    mutationFn: async (body: OrgChannelGuestForm) =>
      orgChannelSettingsSchema.parse(
        await assistantJson(`${root}/org-specialist`, { method: "PUT", body }),
      ),
    onSuccess: (data) => {
      queryClient.setQueryData(key, data);
      void queryClient.invalidateQueries({
        queryKey: ["assistant", "nyxagent", userId, "channels"],
      });
    },
  });
  const issue = useMutation({
    mutationFn: async () =>
      orgChannelChallengeSchema.parse(
        await assistantJson(`${root}/member-link`, { method: "POST" }),
      ),
  });
  const status = useQuery({
    queryKey: [...key, "challenge", challenge],
    queryFn: async () =>
      orgChannelChallengeSchema.parse(
        await assistantJson(
          `${root}/member-link/${encodeURIComponent(challenge ?? "")}`,
        ),
      ),
    enabled: Boolean(userId && challenge),
    refetchInterval: 3000,
    retry: false,
  });
  const confirm = useMutation({
    mutationFn: () =>
      assistantJson(
        `${root}/member-link/${encodeURIComponent(challenge ?? "")}/confirm`,
        { method: "POST" },
      ),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: key });
    },
  });
  const revoke = useMutation({
    mutationFn: () =>
      assistantJson(`${root}/member-link`, { method: "DELETE" }),
    onSuccess: () => { void queryClient.invalidateQueries({ queryKey: key }); },
  });
  return { settings, save, issue, status, confirm, revoke };
}
