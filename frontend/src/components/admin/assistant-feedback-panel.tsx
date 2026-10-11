import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api-client";

type FeedbackAdmin = { counts: readonly { day: string; up: number; down: number }[]; recent: readonly { message_id: string; vote: string; comment?: string | null; created_at: string; }[]; next_cursor?: string | null };
export function AssistantFeedbackPanel() {
  const query = useQuery({ queryKey: ["admin", "assistant-feedback"], queryFn: () => api.get<FeedbackAdmin>("/admin/assistant/feedback") });
  if (query.isLoading) return <div className="rounded-xl border border-border/50 bg-card p-4 text-12 text-muted-foreground">Loading assistant feedback…</div>;
  if (query.isError) return null;
  const data = query.data;
  return <section aria-labelledby="assistant-feedback-heading" className="mb-6 rounded-xl border border-border/50 bg-card p-4">
    <h2 id="assistant-feedback-heading" className="text-15 font-semibold">Assistant reply feedback</h2>
    <div className="mt-3 grid grid-cols-2 gap-3 sm:grid-cols-4">{data?.counts.slice(0, 4).map((row) => <div key={row.day} className="rounded-lg bg-overlay p-3 text-center"><div className="text-11 text-muted-foreground">{row.day}</div><div className="mt-1 text-13">👍 {row.up} · 👎 {row.down}</div></div>)}</div>
    <div className="mt-4 space-y-2">{data?.recent.slice(0, 10).map((row) => <div key={`${row.message_id}-${row.created_at}`} className="flex items-start justify-between gap-3 border-t border-border/50 pt-2 text-11"><span className="font-mono text-text-tertiary">{row.message_id}</span><span>{row.vote === "up" ? "👍" : "👎"}{row.comment ? ` ${row.comment}` : ""}</span></div>)}</div>
  </section>;
}
