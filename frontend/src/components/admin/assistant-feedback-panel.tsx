import { ApiError } from "@/lib/api-client";
import { useMemo, useState } from "react";
import {
  ChevronLeft,
  ChevronRight,
  MessageSquare,
  ThumbsDown,
  ThumbsUp,
} from "lucide-react";
import {
  useAssistantFeedbackAdmin,
  useFeedbackThread,
} from "@/hooks/use-assistant-feedback-admin";
import type { FeedbackReplyContext } from "@/schemas/assistant-feedback-admin";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

/** Only metadata and the submitted comment; never prompt/reply text. */
function ReplyContext({
  context,
}: {
  context: FeedbackReplyContext | null | undefined;
}) {
  if (!context)
    return (
      <span className="text-11 text-text-tertiary">
        Reply details unavailable
      </span>
    );
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-11 text-text-tertiary">
      <span>
        {context.agent_kind === "nyxbot"
          ? "NyxBot"
          : context.agent_kind === "specialist"
            ? "Specialist"
            : "Agent unspecified"}
      </span>
      {context.model && <span className="break-all">{context.model}</span>}
      {context.latency_ms != null && (
        <span>{(context.latency_ms / 1000).toFixed(1)} s reply time</span>
      )}
      {Boolean(context.tool_calls) && (
        <span>
          {context.tool_calls} recorded tools · {context.tool_errors ?? 0}{" "}
          {context.tool_errors === 1 ? "error" : "errors"}
        </span>
      )}
      {context.error_code && (
        <span className="text-destructive">{context.error_code}</span>
      )}
    </div>
  );
}

export function AssistantFeedbackPanel({ range }: { range?: URLSearchParams }) {
  return <FeedbackList key={range?.toString() ?? ""} range={range} />;
}
function FeedbackList({ range }: { range?: URLSearchParams }) {
  const [vote, setVote] = useState("all");
  const [model, setModel] = useState("");
  const [agentKind, setAgentKind] = useState("all");
  const [cursors, setCursors] = useState<(string | undefined)[]>([undefined]);
  const [page, setPage] = useState(0);
  const rangeKey = range?.toString() ?? "";
  const params = useMemo(() => {
    const next = new URLSearchParams(rangeKey);
    if (vote !== "all") next.set("vote", vote);
    if (model.trim()) next.set("model", model.trim());
    if (agentKind !== "all") next.set("agent_kind", agentKind);
    if (cursors[page]) next.set("cursor", cursors[page]!);
    return next;
  }, [rangeKey, vote, model, agentKind, cursors, page]);
  const query = useAssistantFeedbackAdmin(params);
  const denied =
    query.error instanceof ApiError && [403, 404].includes(query.error.status);
  const data = denied ? undefined : query.data;
  const counts = data?.counts.reduce(
    (sum, day) => ({ up: sum.up + day.up, down: sum.down + day.down }),
    { up: 0, down: 0 },
  );
  function resetPage() {
    setCursors([undefined]);
    setPage(0);
  }
  const filtered = vote !== "all" || Boolean(model) || agentKind !== "all";
  return (
    <Card aria-labelledby="assistant-feedback-heading">
      <div className="space-y-4 border-b border-border/50 p-4">
        <div>
          <h2
            id="assistant-feedback-heading"
            className="font-display text-15 font-semibold leading-none tracking-tight"
          >
            Recent feedback
          </h2>
          <p className="mt-1 text-12 text-muted-foreground">
            Read what users found helpful and what needs attention.
          </p>
        </div>
        {counts && (
          <div
            className="flex flex-wrap gap-4 text-12"
            aria-label="Matching feedback totals"
          >
            <span>
              <strong className="tabular-nums">
                {counts.up.toLocaleString()}
              </strong>{" "}
              helpful
            </span>
            <span>
              <strong className="tabular-nums">
                {counts.down.toLocaleString()}
              </strong>{" "}
              not helpful
            </span>
            <span className="text-text-tertiary">
              All matching ratings in this period, across every page
            </span>
          </div>
        )}
        <div className="flex flex-wrap items-end gap-3">
          <label className="w-36 space-y-1 text-11 text-muted-foreground">
            Vote
            <Select
              value={vote}
              onValueChange={(value) => {
                resetPage();
                setVote(value);
              }}
            >
              <SelectTrigger aria-label="Vote">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="all">All feedback</SelectItem>
                <SelectItem value="up">Helpful</SelectItem>
                <SelectItem value="down">Not helpful</SelectItem>
              </SelectContent>
            </Select>
          </label>
          <label className="w-36 space-y-1 text-11 text-muted-foreground">
            Agent
            <Select
              value={agentKind}
              onValueChange={(value) => {
                resetPage();
                setAgentKind(value);
              }}
            >
              <SelectTrigger aria-label="Agent">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="all">All agents</SelectItem>
                <SelectItem value="nyxbot">NyxBot</SelectItem>
                <SelectItem value="specialist">Specialist</SelectItem>
              </SelectContent>
            </Select>
          </label>
          <form
            className="flex items-end gap-2"
            onSubmit={(event) => {
              event.preventDefault();
              const value = new FormData(event.currentTarget).get("model");
              resetPage();
              setModel(typeof value === "string" ? value.trim() : "");
            }}
          >
            <label className="space-y-1 text-11 text-muted-foreground">
              Model
              <Input
                key={model}
                name="model"
                placeholder="Any model"
                defaultValue={model}
                className="w-44"
              />
            </label>
            <Button type="submit" variant="outline">
              Apply
            </Button>
          </form>
          {filtered && (
            <Button
              variant="ghost"
              onClick={() => {
                resetPage();
                setVote("all");
                setAgentKind("all");
                setModel("");
              }}
            >
              Clear filters
            </Button>
          )}
        </div>
        <p className="text-11 text-text-tertiary">
          These filters apply to the feedback list. Charts show all feedback in
          the selected period.
        </p>
      </div>
      <div aria-busy={query.isFetching}>
        {query.isFetching && !query.isPending && (
          <p role="status" className="px-4 pt-3 text-11 text-muted-foreground">
            Updating feedback…
          </p>
        )}
        {query.isError ? (
          <div
            role="alert"
            className="flex items-center justify-between gap-3 p-4 text-12"
          >
            <span>
              {data
                ? "Unable to load assistant feedback. Matching totals are from the previous successful refresh."
                : "Unable to load assistant feedback."}
            </span>
            <Button variant="outline" onClick={() => void query.refetch()}>
              Retry
            </Button>
          </div>
        ) : query.isPending ? (
          <div aria-label="Loading feedback" className="space-y-3 p-4">
            <Skeleton className="h-20" />
            <Skeleton className="h-20" />
          </div>
        ) : data?.recent.length ? (
          <ul className="divide-y divide-border/50">
            {data.recent.map((row) => {
              const Icon = row.vote === "up" ? ThumbsUp : ThumbsDown;
              return (
                <li
                  key={row.id ?? `${row.message_id}-${row.created_at}`}
                  className="p-4"
                >
                  <div className="flex flex-wrap items-center justify-between gap-2">
                    <Badge
                      variant={row.vote === "up" ? "success" : "destructive"}
                    >
                      <Icon className="mr-1 size-3" aria-hidden="true" />
                      {row.vote === "up" ? "Helpful" : "Not helpful"}
                    </Badge>
                    <time
                      dateTime={row.created_at}
                      className="font-mono text-11 text-text-tertiary"
                    >
                      {new Date(row.created_at).toLocaleString()}
                    </time>
                  </div>
                  <p
                    className={`my-3 whitespace-pre-wrap break-words text-13 ${row.comment ? "text-foreground" : "text-text-tertiary"}`}
                  >
                    {row.comment || "No comment provided"}
                  </p>
                  <ReplyContext context={row.context} />
                  {data.can_preview_threads && row.id && (
                    <FeedbackThread
                      key={`${row.id}-${row.updated_at}`}
                      id={row.id}
                      revision={row.updated_at}
                      disabled={query.isPlaceholderData}
                    />
                  )}
                  {!data.can_preview_threads && (
                    <p className="mt-2 text-11 text-text-tertiary">
                      Thread previews require Admin or Operator access.
                    </p>
                  )}
                  <details className="mt-2 text-11 text-text-tertiary">
                    <summary className="w-fit cursor-pointer">
                      Reply details
                    </summary>
                    <dl className="mt-2 space-y-1 break-all">
                      <div>
                        <dt className="inline">Message ID: </dt>
                        <dd className="inline font-mono">{row.message_id}</dd>
                      </div>
                      {row.context?.turn_id && (
                        <div>
                          <dt className="inline">Turn ID: </dt>
                          <dd className="inline font-mono">
                            {row.context.turn_id}
                          </dd>
                        </div>
                      )}
                      {Boolean(row.context?.tool_labels?.length) && (
                        <div>
                          <dt className="inline">Recorded tools: </dt>
                          <dd className="inline font-mono">
                            {row.context?.tool_labels?.join(", ")}
                          </dd>
                        </div>
                      )}
                    </dl>
                  </details>
                </li>
              );
            })}
          </ul>
        ) : (
          <div className="m-4 rounded-lg border border-dashed border-border px-4 py-8 text-center">
            <MessageSquare
              className="mx-auto mb-3 size-5 text-text-tertiary"
              aria-hidden="true"
            />
            <p className="text-13 font-medium">
              {filtered
                ? "No feedback matches these filters."
                : "No feedback in this period."}
            </p>
            <p className="mt-1 text-12 text-muted-foreground">
              {filtered
                ? "Try another vote, agent, or model."
                : "Votes and comments appear here when users rate a reply."}
            </p>
          </div>
        )}
      </div>
      {(page > 0 || data?.next_cursor) && (
        <div className="flex items-center justify-between border-t border-border/50 px-4 py-3">
          <span className="text-11 text-text-tertiary">
            Page {page + 1} · up to 100 ratings per page
          </span>
          <div className="flex gap-2">
            <Button
              variant="outline"
              disabled={page === 0 || query.isFetching}
              onClick={() => setPage(page - 1)}
            >
              <ChevronLeft className="size-3" />
              Previous page
            </Button>
            <Button
              variant="outline"
              disabled={!data?.next_cursor || query.isFetching}
              onClick={() => {
                setCursors([
                  ...cursors.slice(0, page + 1),
                  data?.next_cursor ?? undefined,
                ]);
                setPage(page + 1);
              }}
            >
              Next page
              <ChevronRight className="size-3" />
            </Button>
          </div>
        </div>
      )}
    </Card>
  );
}

function FeedbackThread({
  id,
  revision,
  disabled,
}: {
  id: string;
  revision: string;
  disabled: boolean;
}) {
  const [open, setOpen] = useState(false);
  return (
    <div className="mt-3">
      <Button
        variant="outline"
        aria-expanded={open}
        disabled={disabled}
        onClick={() => setOpen(!open)}
      >
        {open ? "Hide message thread" : "Preview message thread"}
      </Button>
      {open && <ThreadMessages id={id} revision={revision} />}
    </div>
  );
}
function ThreadMessages({ id, revision }: { id: string; revision: string }) {
  const query = useFeedbackThread(id, revision, true);
  const inaccessible =
    query.error instanceof ApiError && [403, 404].includes(query.error.status);
  const unavailable =
    inaccessible || query.data?.pages.some((page) => page.unavailable);
  const messages = [...(query.data?.pages ?? [])]
    .reverse()
    .flatMap((page) => page.messages);
  const unique = messages.filter(
    (m, i) => messages.findIndex((other) => other.id === m.id) === i,
  );
  if (unavailable)
    return (
      <p role="status" className="mt-3 text-12 text-muted-foreground">
        This message thread is no longer available.
      </p>
    );
  return (
    <section
      className="mt-3 space-y-3 rounded-lg border border-border/50 p-3"
      aria-label="Feedback message thread"
      aria-busy={query.isFetching}
    >
      <p className="text-11 text-text-tertiary">
        Conversation through the rated reply. Text excerpts only. Known secret
        formats are redacted.
      </p>
      {query.isPending && (
        <Skeleton aria-label="Loading message thread" className="h-24" />
      )}
      {query.isError && (
        <div role="alert" className="text-12">
          <p>
            Could not load message context.
            {unique.length
              ? " Previously loaded excerpts may be out of date."
              : ""}
          </p>
          <Button
            variant="outline"
            onClick={() =>
              void (query.isFetchNextPageError
                ? query.fetchNextPage()
                : query.refetch())
            }
          >
            Retry context
          </Button>
        </div>
      )}
      {query.hasNextPage && (
        <Button
          variant="outline"
          disabled={query.isFetching}
          onClick={() => void query.fetchNextPage()}
        >
          {query.isFetchingNextPage
            ? "Loading previous messages…"
            : "Load previous messages"}
        </Button>
      )}
      <ol className="space-y-3">
        {unique.map((message) => (
          <li
            key={message.id}
            className={`rounded-lg p-3 ${message.rated ? "border border-hairline-strong bg-overlay" : "border border-border/50"}`}
          >
            <div className="mb-2 flex flex-wrap items-center justify-between gap-2 text-11 text-text-tertiary">
              <strong className="text-foreground">
                {message.role === "user" ? "User" : "Assistant"}
                {message.rated ? " · Rated reply" : ""}
              </strong>
              <time dateTime={message.created_at}>
                {new Date(message.created_at).toLocaleString()}
              </time>
            </div>
            <p className="whitespace-pre-wrap break-words text-12">
              {message.text || "No text available."}
            </p>
            {message.truncated && (
              <p className="mt-1 text-11 text-text-tertiary">
                Message excerpt truncated.
              </p>
            )}
          </li>
        ))}
      </ol>
      {query.isSuccess && !unique.length && (
        <p className="text-12 text-muted-foreground">
          No retained messages available.
        </p>
      )}
    </section>
  );
}
