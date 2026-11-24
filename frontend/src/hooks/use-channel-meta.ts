import { useMutation, useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  metaStartedSchema,
  metaProgressSchema,
  metaConnectedSchema,
} from "@/schemas/channel-meta";
import { useAuthStore } from "@/stores/auth-store";
import { useChannelConnectLinkContext } from "./use-channel-connect-link";

export function useMetaConnection(platform: string, sessionId: string | null) {
  const actor = useAuthStore((s) => s.user?.id);
  const link = useChannelConnectLinkContext();
  const start = useMutation({
    retry: false,
    gcTime: 0,
    mutationFn: async (input: {
      label: string;
      target_org_id?: string;
      reconnect_bot_id?: string;
      setup_token?: string;
    }) => {
      const started = metaStartedSchema.parse(
        await api.post(`/channel-bots/meta/${platform}/start`, input),
      );
      // A poll in this tab can still describe an earlier start from another tab.
      // Read the tracked link after the server has committed this start.
      if (link && (await link.refresh()) !== started.session_id) return null;
      return started;
    },
  });
  const progress = useQuery({
    queryKey: ["meta-connection", actor, sessionId],
    enabled: Boolean(actor && sessionId),
    retry: false,
    gcTime: 0,
    queryFn: async () =>
      metaProgressSchema.parse(
        await api.get(`/channel-bots/meta-sessions/${sessionId}`),
      ),
    refetchInterval: (query) =>
      query.state.error
        ? false
        : ["pending", "exchanging", "installing"].includes(
              query.state.data?.status ?? "pending",
            )
          ? 1500
          : false,
  });
  const finish = useMutation({
    retry: false,
    gcTime: 0,
    mutationFn: async (input: {
      asset_id: string;
      exclusive_routing_verified: boolean;
    }) =>
      metaConnectedSchema.parse(
        await api.post(
          `/channel-bots/meta-sessions/${sessionId}/finish`,
          input,
        ),
      ),
  });
  return { start, progress, finish };
}

export interface MetaCustomerSetupResult {
  channel_id: string;
  agent_id?: string;
  guests_enabled: boolean;
  owner_link_code?: string | null;
  owner_link_expires_at?: string | null;
  readiness: {
    guest_readiness?: { status: string; fix?: string | null };
    grants_ready?: boolean;
    readiness_reason?: string | null;
  };
}
export function useMetaCustomerSetup(botId: string) {
  return useMutation({
    retry: false,
    gcTime: 0,
    mutationFn: (input: {
      agent_id: string;
      action:
        | "check"
        | "assign"
        | "prepare_model"
        | "enable_guests"
        | "owner_link";
    }) =>
      api.post<MetaCustomerSetupResult>(
        `/channel-bots/${botId}/customer-setup`,
        input,
      ),
  });
}
