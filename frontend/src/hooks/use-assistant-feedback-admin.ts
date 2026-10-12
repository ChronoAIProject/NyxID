import { useQuery } from "@tanstack/react-query";
import { z } from "zod";
import { api } from "@/lib/api-client";
const schema = z.object({
  counts: z.array(
    z.object({ day: z.string(), up: z.number(), down: z.number() }),
  ),
  recent: z.array(
    z.object({
      message_id: z.string(),
      vote: z.enum(["up", "down"]),
      comment: z.string().nullable().optional(),
      created_at: z.string(),
      updated_at: z.string(),
    }),
  ),
  next_cursor: z.string().nullable().optional(),
});
export function useAssistantFeedbackAdmin(params: URLSearchParams) {
  return useQuery({
    queryKey: ["admin", "assistant-feedback", params.toString()],
    queryFn: async () =>
      schema.parse(await api.get(`/admin/assistant/feedback?${params}`)),
    retry: false,
  });
}
