import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { assistantFeedbackAdminSchema } from "@/schemas/assistant-feedback-admin";

export function useAssistantFeedbackAdmin(params: URLSearchParams) {
  return useQuery({
    queryKey: ["admin", "assistant-feedback", params.toString()],
    queryFn: async () =>
      assistantFeedbackAdminSchema.parse(
        await api.get(`/admin/assistant/feedback?${params}`),
      ),
    retry: false,
    // Keep the filters and the previous page on screen while a new filter loads.
    placeholderData: keepPreviousData,
  });
}
