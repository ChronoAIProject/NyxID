import { useMemo, useState } from "react";
import { useAssistantFeedbackAdmin } from "@/hooks/use-assistant-feedback-admin";

export function AssistantFeedbackPanel() {
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const [vote, setVote] = useState("");
  const [cursor, setCursor] = useState<string>();
  const params = useMemo(() => {
    const next = new URLSearchParams();
    if (from) next.set("from", new Date(`${from}T00:00:00Z`).toISOString());
    if (to) next.set("to", new Date(`${to}T23:59:59Z`).toISOString());
    if (vote) next.set("vote", vote);
    if (cursor) next.set("cursor", cursor);
    return next;
  }, [from, to, vote, cursor]);
  const query = useAssistantFeedbackAdmin(params);
  const data = query.data;
  const error = query.error ? "Unable to load assistant feedback." : undefined;
  /* TanStack Query owns fetch lifecycle, schema validation, and cache identity. */
  if (query.isPending)
    return (
      <section
        aria-label="Assistant reply feedback"
        className="mb-6 rounded-xl border border-border/50 bg-card p-4"
      >
        Loading assistant feedback…
      </section>
    );
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
              {row.context
                ? ` ${row.context.agent_kind ?? ""} ${row.context.model ?? ""}`
                : " no context (pre-A1 row)"}
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
