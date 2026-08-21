import { useQuery } from "@tanstack/react-query";
import { assistantWireLogRecordSchema } from "@/schemas/assistant-wire-log";
import type { AssistantWireLogRecord } from "@/schemas/assistant-wire-log";
import { ApiError } from "@/lib/api-client";
import { assistantApi } from "@/lib/assistant/aevatar-transport";

export type AssistantWireLogResult =
  | {
      readonly status: "loaded";
      readonly record: AssistantWireLogRecord;
    }
  | { readonly status: "expired" };

async function fetchAssistantWireLog(
  wireLogId: string,
): Promise<AssistantWireLogResult> {
  try {
    const response = await assistantApi.get<unknown>(
      `/assistant/wire-logs/${encodeURIComponent(wireLogId)}`,
    );
    return {
      status: "loaded",
      record: assistantWireLogRecordSchema.parse(response),
    };
  } catch (error) {
    if (error instanceof ApiError && error.status === 404) {
      return { status: "expired" };
    }
    throw error;
  }
}

export function useAssistantWireLog(
  wireLogId: string | null,
  enabled: boolean,
) {
  return useQuery({
    queryKey: ["assistant-wire-log", wireLogId],
    queryFn: () => {
      if (!wireLogId) {
        throw new Error("A wire log id is required.");
      }
      return fetchAssistantWireLog(wireLogId);
    },
    enabled: enabled && Boolean(wireLogId),
    staleTime: Infinity,
    retry: false,
  });
}
