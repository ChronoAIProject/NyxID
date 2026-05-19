import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";

export interface BrokerBindingExternalSubject {
  readonly platform: string;
  readonly tenant?: string;
  readonly external_user_id: string;
}

export interface BrokerBindingListItem {
  readonly binding_hash: string;
  readonly client_id: string;
  readonly client_name: string | null;
  readonly external_subject: BrokerBindingExternalSubject | null;
  readonly scopes: readonly string[];
  readonly created_at: string;
  readonly last_used_at: string | null;
}

export interface BrokerBindingListResponse {
  readonly bindings: readonly BrokerBindingListItem[];
}

export function useMyBrokerBindings() {
  return useQuery({
    queryKey: ["broker-bindings", "me"],
    queryFn: async (): Promise<BrokerBindingListResponse> => {
      return api.get<BrokerBindingListResponse>("/users/me/broker-bindings");
    },
  });
}

export function useRevokeBrokerBinding() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (bindingHash: string): Promise<void> => {
      await api.delete<void>(`/users/me/broker-bindings/${bindingHash}`);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: ["broker-bindings", "me"],
      });
    },
  });
}
