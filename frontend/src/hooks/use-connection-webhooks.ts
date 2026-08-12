import { useMutation, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  connectionWebhookSecretResponseSchema,
  type ConnectionWebhookSecretResponse,
} from "@/schemas/connection-webhooks";
import type { OAuthClient } from "@/types/api";

function connectionWebhookPath(clientId: string): string {
  return `/developer/oauth-clients/${encodeURIComponent(clientId)}/connection-webhook`;
}

function useInvalidateDeveloperApps() {
  const queryClient = useQueryClient();
  return () =>
    queryClient.invalidateQueries({
      queryKey: ["developer", "oauth-clients"],
    });
}

export function useConfigureConnectionWebhook() {
  const invalidate = useInvalidateDeveloperApps();
  return useMutation({
    mutationFn: async ({
      clientId,
      url,
    }: {
      readonly clientId: string;
      readonly url: string;
    }): Promise<ConnectionWebhookSecretResponse> => {
      const response = await api.put<ConnectionWebhookSecretResponse>(
        connectionWebhookPath(clientId),
        { url },
      );
      return connectionWebhookSecretResponseSchema.parse(response);
    },
    onSuccess: () => {
      void invalidate();
    },
  });
}

export function useRotateConnectionWebhookSecret() {
  const invalidate = useInvalidateDeveloperApps();
  return useMutation({
    mutationFn: async (
      clientId: string,
    ): Promise<ConnectionWebhookSecretResponse> => {
      const response = await api.post<ConnectionWebhookSecretResponse>(
        `${connectionWebhookPath(clientId)}/rotate-secret`,
      );
      return connectionWebhookSecretResponseSchema.parse(response);
    },
    onSuccess: () => {
      void invalidate();
    },
  });
}

export function useDisableConnectionWebhook() {
  const invalidate = useInvalidateDeveloperApps();
  return useMutation({
    mutationFn: async (clientId: string): Promise<OAuthClient> =>
      api.delete<OAuthClient>(connectionWebhookPath(clientId)),
    onSuccess: () => {
      void invalidate();
    },
  });
}
