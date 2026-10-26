import { z } from "zod";

const count = z.number().int().nonnegative();
export const assistantAnalyticsSchema = z.object({
  from: z.iso.datetime({ offset: true }),
  to: z.iso.datetime({ offset: true }),
  generated_at: z.iso.datetime({ offset: true }),
  active_sessions: count,
  active_accounts: count,
  running_now: count,
  outcomes: z.object({
    completed: count,
    failed: count,
    stopped: count,
    with_tools: count,
    with_tool_errors: count,
  }),
  daily_sessions: z.array(z.object({ day: z.string(), sessions: count })),
  daily_turns: z.array(z.object({ day: z.string(), turns: count })),
});
export type AssistantAnalytics = z.infer<typeof assistantAnalyticsSchema>;
