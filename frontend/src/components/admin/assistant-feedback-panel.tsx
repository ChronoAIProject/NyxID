import { useEffect, useState } from "react";
import { api } from "@/lib/api-client";

type FeedbackAdmin = {
  counts: readonly { day: string; up: number; down: number }[];
  recent: readonly {
    message_id: string;
    vote: string;
    comment?: string | null;
    created_at: string;
    updated_at: string;
  }[];
  next_cursor?: string | null;
};
export function AssistantFeedbackPanel() {
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const [vote, setVote] = useState("");
  const [cursor, setCursor] = useState<string>();
  const [data, setData] = useState<FeedbackAdmin>();
  const [error, setError] = useState<string>();
  useEffect(() => {
    const params = new URLSearchParams();
    if (from) params.set("from", new Date(`${from}T00:00:00Z`).toISOString());
    if (to) params.set("to", new Date(`${to}T23:59:59Z`).toISOString());
    if (vote) params.set("vote", vote);
    if (cursor) params.set("cursor", cursor);
    void api
      .get<FeedbackAdmin>(`/admin/assistant/feedback?${params}`)
      .then((next) => {
        setData(next);
        setError(undefined);
      })
      .catch(() => setError("Unable to load assistant feedback."));
  }, [from, to, vote, cursor]);
  return (
    <section
      aria-labelledby="assistant-feedback-heading"
      className="mb-6 rounded-xl border border-border/50 bg-card p-4"
    >
      <h2 id="assistant-feedback-heading" className="text-15 font-semibold">
        Assistant reply feedback
      </h2>
      <div className="mt-3 flex flex-wrap gap-2 text-11">
        <label>
          From{" "}
          <input
            type="date"
            value={from}
            onChange={(e) => {
              setCursor(undefined);
              setFrom(e.target.value);
            }}
          />
        </label>
        <label>
          To{" "}
          <input
            type="date"
            value={to}
            onChange={(e) => {
              setCursor(undefined);
              setTo(e.target.value);
            }}
          />
        </label>
        <label>
          Vote{" "}
          <select
            value={vote}
            onChange={(e) => {
              setCursor(undefined);
              setVote(e.target.value);
            }}
          >
            <option value="">All</option>
            <option value="up">Helpful</option>
            <option value="down">Not helpful</option>
          </select>
        </label>
      </div>
      {error ? (
        <p role="alert" className="mt-3 text-11 text-destructive">
          {error}
        </p>
      ) : null}
      <div className="mt-3 grid grid-cols-2 gap-3 sm:grid-cols-4">
        {(data?.counts ?? []).map((row) => (
          <div key={row.day} className="rounded-lg bg-overlay p-3 text-center">
            <div className="text-11 text-muted-foreground">{row.day}</div>
            <div className="mt-1 text-13">
              {row.up} up · {row.down} down
            </div>
          </div>
        ))}
      </div>
      <div className="mt-4 space-y-2">
        {(data?.recent ?? []).map((row) => (
          <div
            key={`${row.message_id}-${row.created_at}`}
            className="flex items-start justify-between gap-3 border-t border-border/50 pt-2 text-11"
          >
            <span className="font-mono text-text-tertiary">
              {row.message_id}
            </span>
            <time dateTime={row.created_at}>
              {new Date(row.created_at).toLocaleString()}
            </time>
            <span>
              {row.vote}
              {row.comment ? ` ${row.comment}` : ""}
            </span>
          </div>
        ))}
      </div>
      {data?.next_cursor ? (
        <button
          type="button"
          className="mt-3 text-11 underline"
          onClick={() => setCursor(data.next_cursor ?? undefined)}
        >
          Next page
        </button>
      ) : null}
    </section>
  );
}
