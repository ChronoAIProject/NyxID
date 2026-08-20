import { useMutation, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";

export interface AssistantKeyResource {
  readonly keyId: string;
}

export interface AssistantKeyMutationResponse {
  readonly resource: AssistantKeyResource;
  readonly replayed: boolean;
}

export interface AssistantKeyBindResponse extends AssistantKeyMutationResponse {
  readonly bindingId: string;
}

export interface ApiKeyAuthorizationEvidence {
  readonly id: string;
  readonly name: string;
  readonly scopes: string;
  readonly platform: string | null;
  readonly is_active: boolean;
  readonly allowed_service_ids: readonly string[];
  readonly allow_all_services: boolean;
  readonly allowed_node_ids: readonly string[];
  readonly allow_all_nodes: boolean;
  readonly created_at: string;
  readonly rotation_predecessor_id?: string | null;
  readonly state_version?: number;
  readonly updated_at?: string | null;
}

export interface BindingAuthorizationEvidence {
  readonly id: string;
  readonly api_key_id: string;
  readonly user_service_id: string;
  readonly user_api_key_id: string;
  readonly created_at: string;
  readonly updated_at: string;
}

function invalidateKeyQueries(
  queryClient: ReturnType<typeof useQueryClient>,
  keyId: string,
) {
  void queryClient.invalidateQueries({
    predicate: (query) =>
      Array.isArray(query.queryKey) && query.queryKey[0] === "api-keys",
  });
  void queryClient.invalidateQueries({
    queryKey: ["agent-bindings", keyId],
  });
}

export function useAssistantKeyUpdate() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (body: {
      readonly actionRequestId: string;
      readonly keyId: string;
      readonly name?: string;
      readonly platform?: string;
      readonly description?: string;
      readonly expectedStateVersion?: number;
    }): Promise<AssistantKeyMutationResponse> => {
      return api.post<AssistantKeyMutationResponse>(
        "/assistant/actions/keys/update",
        body,
      );
    },
    onSuccess: (_data, variables) => {
      invalidateKeyQueries(queryClient, variables.keyId);
    },
  });
}

export function useAssistantKeyDelete() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (body: {
      readonly actionRequestId: string;
      readonly keyId: string;
      readonly expectedStateVersion: number;
    }): Promise<AssistantKeyMutationResponse> => {
      return api.post<AssistantKeyMutationResponse>(
        "/assistant/actions/keys/delete",
        body,
      );
    },
    onSuccess: (_data, variables) => {
      invalidateKeyQueries(queryClient, variables.keyId);
    },
  });
}

export function useAssistantKeyExtendScope() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (body: {
      readonly actionRequestId: string;
      readonly keyId: string;
      readonly addServiceIds: readonly string[];
      readonly expectedStateVersion?: number;
    }): Promise<AssistantKeyMutationResponse> => {
      return api.post<AssistantKeyMutationResponse>(
        "/assistant/actions/keys/extend-scope",
        body,
      );
    },
    onSuccess: (_data, variables) => {
      invalidateKeyQueries(queryClient, variables.keyId);
    },
  });
}

export function useAssistantKeyBindCredential() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (body: {
      readonly actionRequestId: string;
      readonly keyId: string;
      readonly userServiceId: string;
      readonly externalKeyId: string;
    }): Promise<AssistantKeyBindResponse> => {
      return api.post<AssistantKeyBindResponse>(
        "/assistant/actions/keys/bind-credential",
        body,
      );
    },
    onSuccess: (_data, variables) => {
      invalidateKeyQueries(queryClient, variables.keyId);
    },
  });
}

export function readApiKeyAuthorization(
  keyId: string,
): Promise<ApiKeyAuthorizationEvidence> {
  return api.get<ApiKeyAuthorizationEvidence>(
    `/api-keys/${encodeURIComponent(keyId)}/authorization`,
  );
}

export function readBindingAuthorization(
  keyId: string,
  userServiceId: string,
): Promise<BindingAuthorizationEvidence> {
  return api.get<BindingAuthorizationEvidence>(
    `/api-keys/${encodeURIComponent(keyId)}/bindings/by-service/${encodeURIComponent(userServiceId)}/authorization`,
  );
}
