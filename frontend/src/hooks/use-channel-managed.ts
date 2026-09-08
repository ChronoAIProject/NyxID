import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { z } from "zod";
import { api, apiFetch, ApiError } from "@/lib/api-client";
import { SsePayloadDecoder } from "@/lib/assistant/sse-frame-normalizer";
import {
  managedBootstrapSchema,
  managedCompleteSchema,
  type ManagedCompleteInput,
} from "@/schemas/channel-managed";
import type { CreateChannelBotResponse } from "@/types/channels";

export function useManagedOnboarding(platform: string, enabled = true) {
  return useQuery({
    queryKey: ["managed-onboarding", platform],
    enabled,
    retry: false,
    staleTime: 0,
    queryFn: async () =>
      managedBootstrapSchema.parse(
        await api.get(
          `/channel-bots/managed-onboarding/${encodeURIComponent(platform)}`,
        ),
      ),
  });
}

const progressSchema = z.object({
  stage: z.enum(["exchanging", "subscribing", "registering"]).optional(),
  error: z.string().optional(),
  message: z.string().optional(),
  error_code: z.number().optional(),
  result: z
    .object({ id: z.string(), platform: z.string() })
    .passthrough()
    .optional(),
});
export async function completeManagedOnboarding(
  platform: string,
  input: ManagedCompleteInput,
  onStage: (stage: string) => void,
  signal: AbortSignal,
  apiBaseUrl: string,
): Promise<CreateChannelBotResponse> {
  const response = await apiFetch(
    `/channel-bots/managed-onboarding/${encodeURIComponent(platform)}/complete`,
    {
      method: "POST",
      apiBaseUrl,
      headers: {
        "Content-Type": "application/json",
        Accept: "text/event-stream",
      },
      body: managedCompleteSchema.parse(input),
      signal,
    },
  );
  if (!response.headers.get("content-type")?.includes("text/event-stream"))
    return (await response.json()) as CreateChannelBotResponse;
  if (!response.body)
    throw new Error(
      "Meta onboarding response was interrupted. Check your bot list before reconnecting.",
    );
  const decoder = new SsePayloadDecoder();
  const reader = response.body.getReader();
  try {
    while (true) {
      const { done, value } = await reader.read();
      for (const payload of done ? decoder.finish() : decoder.push(value)) {
        const event = progressSchema.parse(JSON.parse(payload));
        if (event.error)
          throw new ApiError(response.status, {
            error: event.error,
            message: event.message ?? event.error,
            error_code: event.error_code ?? -1,
          });
        if (event.stage) onStage(event.stage);
        if (event.result)
          return event.result as unknown as CreateChannelBotResponse;
      }
      if (done)
        throw new Error(
          "Meta onboarding response was interrupted. Check your bot list before reconnecting.",
        );
    }
  } finally {
    reader.releaseLock();
  }
}

export function useReregisterChannelBot() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (id: string) =>
      api.post(`/channel-bots/${encodeURIComponent(id)}/reregister`),
    onSettled: () => client.invalidateQueries({ queryKey: ["channel-bots"] }),
  });
}

export function useRepairChannelBot() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (id: string) =>
      api.post(`/channel-bots/${encodeURIComponent(id)}/managed-setup/repair`),
    onSettled: () => client.invalidateQueries({ queryKey: ["channel-bots"] }),
  });
}
