import type { AssistantAnalytics } from "@/schemas/assistant-analytics";
import type { AssistantFeedbackAdmin } from "@/schemas/assistant-feedback-admin";

export function nyxbotDateRange(days: number, now = new Date()) {
  const end = now.toISOString().slice(0, 10);
  const start = new Date(`${end}T00:00:00Z`);
  start.setUTCDate(start.getUTCDate() - days + 1);
  return { from: start.toISOString().slice(0, 10), to: end };
}

export function nyxbotWindow(from: string, to: string) {
  const start = Date.parse(`${from}T00:00:00Z`);
  const end = Date.parse(`${to}T00:00:00Z`) + 86_400_000;
  if (
    !Number.isFinite(start) ||
    !Number.isFinite(end) ||
    end <= start ||
    end - start > 31 * 86_400_000
  )
    return null;
  return {
    from: new Date(start).toISOString(),
    to: new Date(end).toISOString(),
  };
}

export function nyxbotDailyData(
  from: string,
  to: string,
  analytics?: AssistantAnalytics,
  feedback?: AssistantFeedbackAdmin,
) {
  const sessions = new Map(
    analytics?.daily_sessions.map((row) => [row.day, row.sessions]),
  );
  const turns = new Map(
    analytics?.daily_turns.map((row) => [row.day, row.turns]),
  );
  const votes = new Map(feedback?.counts.map((row) => [row.day, row]));
  const rows = [];
  for (let at = Date.parse(from); at < Date.parse(to); at += 86_400_000) {
    const day = new Date(at).toISOString().slice(0, 10);
    rows.push({
      day,
      sessions: sessions.get(day) ?? 0,
      turns: turns.get(day) ?? 0,
      up: votes.get(day)?.up ?? 0,
      down: votes.get(day)?.down ?? 0,
    });
  }
  return rows;
}
