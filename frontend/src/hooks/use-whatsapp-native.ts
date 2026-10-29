import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type { WhatsappNativeForm } from "@/schemas/whatsapp-native";

export interface WhatsappNativeSettings {
  readonly enabled: boolean;
  readonly reason: string | null;
  readonly meta_app_id: string | null;
  readonly waba_id: string | null;
  readonly phone_number_id: string;
  readonly dedicated_app: boolean;
  readonly revision: number;
  readonly callback_phase: "relay" | "awaiting_gateway" | "gateway" | "awaiting_relay" | "unknown";
  readonly callback_error: string | null;
}
const key = (id: string) => ["channel-bots", "native-whatsapp", id] as const;
const path = (id: string) => `/channel-bots/${encodeURIComponent(id)}/native-whatsapp`;
export function useWhatsappNative(id: string) {
  return useQuery({ queryKey: key(id), queryFn: () => api.get<WhatsappNativeSettings>(path(id)) });
}
export function useSaveWhatsappNative(id: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (data: WhatsappNativeForm & { readonly revision: number }) => api.put<WhatsappNativeSettings>(path(id), data),
    onSuccess: (settings) => client.setQueryData(key(id), settings),
  });
}

export interface WhatsappRollbackToken {
  readonly verify_token: string;
  readonly relay_webhook_url: string;
  readonly revision: number;
  readonly previous_token_invalidated_immediately: boolean;
}

export function useReplaceWhatsappRelayToken(id: string, onIssued: (token: WhatsappRollbackToken) => void) {
  const client = useQueryClient();
  return useMutation({
    gcTime: 0,
    retry: false,
    // Return no secret-bearing mutation data; only the open dialog retains it.
    mutationFn: async (revision: number): Promise<void> => {
      const token = await api.post<WhatsappRollbackToken>(`${path(id)}/relay-verify-token`, { revision });
      onIssued(token);
    },
    onSettled: () => client.invalidateQueries({ queryKey: key(id) }),
  });
}
