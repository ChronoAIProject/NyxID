import { getFormattingLocale } from "@/lib/formatting-locale";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";

import { useMemo, useState } from "react";
import { useAssistantFeedbackAdmin } from "@/hooks/use-assistant-feedback-admin";
import type { FeedbackReplyContext } from "@/schemas/assistant-feedback-admin";

/** Reply metadata beside a vote. Never reply or prompt text. */
function ReplyContext({ context }: { readonly context: FeedbackReplyContext | null | undefined }) {
  const { t } = useUiTranslation();
  if (!context) {
    return <span className="text-text-tertiary">{t("No context (pre-A1 row)")}</span>;
  }
  const facts = [
    context.agent_kind ? `Agent: ${context.agent_kind}` : null,
    context.model ? `Model: ${context.model}` : null,
    context.error_code ? `Error: ${context.error_code}` : null,
    context.latency_ms != null ? `Latency: ${(context.latency_ms / 1000).toFixed(1)} s` : null,
    context.tool_calls
      ? `Tools: ${context.tool_calls} (${context.tool_errors ?? 0} failed)`
      : null,
  ].filter((fact): fact is string => fact !== null);
  return (
    <span className="flex flex-col gap-0.5">
      <span>{facts.join(" · ") || t("No reply metadata")}</span>
      {context.tool_labels?.length ? (
        <span className="font-mono text-text-tertiary">{context.tool_labels.join(", ")}</span>
      ) : null}
    </span>
  );
}

export function AssistantFeedbackPanel() {
  const { t } = useUiTranslation();
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const [vote, setVote] = useState("");
  const [model, setModel] = useState("");
  const [agentKind, setAgentKind] = useState("");
  const [hasError, setHasError] = useState(false);
  const [cursor, setCursor] = useState<string>();
  const params = useMemo(() => {
    const next = new URLSearchParams();
    if (from) next.set("from", new Date(`${from}T00:00:00Z`).toISOString());
    if (to) next.set("to", new Date(`${to}T23:59:59Z`).toISOString());
    if (vote) next.set("vote", vote);
    if (model.trim()) next.set("model", model.trim());
    if (agentKind) next.set("agent_kind", agentKind);
    if (hasError) next.set("has_error", "true");
    if (cursor) next.set("cursor", cursor);
    return next;
  }, [from, to, vote, model, agentKind, hasError, cursor]);
  const query = useAssistantFeedbackAdmin(params);
  const data = query.data;
  const error = query.error ? "Unable to load assistant feedback." : undefined;
  /* TanStack Query owns fetch lifecycle, schema validation, and cache identity. */
  if (query.isPending)
    return (
      <section
        aria-label={t("Assistant reply feedback")}
        className="mb-6 rounded-xl border border-border/50 bg-card p-4"
      >{t("Loading assistant feedback…")}</section>
    );
  return (
    <section
      aria-labelledby="assistant-feedback-heading"
      className="mb-6 rounded-xl border border-border/50 bg-card p-4"
    >
      <h2 id="assistant-feedback-heading" className="text-15 font-semibold">{t("Assistant reply feedback")}</h2>
      <div className="mt-3 flex flex-wrap gap-2 text-11">
        <label>{t("From")}{" "}
          <input
            type="date"
            value={from}
            onChange={(e) => {
              setCursor(undefined);
              setFrom(e.target.value);
            }}
          />
        </label>
        <label>{t("To")}{" "}
          <input
            type="date"
            value={to}
            onChange={(e) => {
              setCursor(undefined);
              setTo(e.target.value);
            }}
          />
        </label>
        <label>{t("Vote")}{" "}
          <select
            value={vote}
            onChange={(e) => {
              setCursor(undefined);
              setVote(e.target.value);
            }}
          >
            <option value="">{t("All")}</option>
            <option value="up">{t("Helpful")}</option>
            <option value="down">{t("Not helpful")}</option>
          </select>
        </label>
        <label>{t("Model")}{" "}
          <input
            type="text"
            value={model}
            placeholder={t("Any")}
            onChange={(e) => {
              setCursor(undefined);
              setModel(e.target.value);
            }}
          />
        </label>
        <label>{t("Agent")}{" "}
          <select
            value={agentKind}
            onChange={(e) => {
              setCursor(undefined);
              setAgentKind(e.target.value);
            }}
          >
            <option value="">{t("All")}</option>
            <option value="nyxbot">{t("NyxBot")}</option>
            <option value="specialist">{t("Specialist")}</option>
          </select>
        </label>
        <label>
          <input
            type="checkbox"
            checked={hasError}
            onChange={(e) => {
              setCursor(undefined);
              setHasError(e.target.checked);
            }}
          />{" "}{t("Failed replies only")}</label>
      </div>
      {error ? (
        <p role="alert" className="mt-3 text-11 text-destructive">
          {translateUi(error)}
        </p>
      ) : null}
      <div className="mt-3 grid grid-cols-2 gap-3 sm:grid-cols-4">
        {(data?.counts ?? []).map((row) => (
          <div key={row.day} className="rounded-lg bg-overlay p-3 text-center">
            <div className="text-11 text-muted-foreground">{row.day}</div>
            <div className="mt-1 text-13">
              {row.up}{t(" up · ")}{row.down}{t(" down")}</div>
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
              {new Date(row.created_at).toLocaleString(getFormattingLocale())}
            </time>
            <span>
              {row.vote === "up" ? t("Helpful") : t("Not helpful")}
              {row.comment ? t(": {{comment}}", { comment: row.comment }) : ""}
            </span>
            <ReplyContext context={row.context} />
          </div>
        ))}
      </div>
      {data?.next_cursor ? (
        <button
          type="button"
          className="mt-3 text-11 underline"
          onClick={() => setCursor(data.next_cursor ?? undefined)}
        >{t("Next page")}</button>
      ) : null}
    </section>
  );
}
