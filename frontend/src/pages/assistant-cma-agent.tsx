import { useState } from "react";
import { Link, useParams, useSearch } from "@tanstack/react-router";
import { Activity, Box, History, Monitor, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { CmaDesktopPanel } from "@/components/assistant/cma-desktop-panel";
import { useFeature } from "@/hooks/use-feature-flag";
import {
  useCmaAgent,
  useCmaResponse,
  useCmaTranscript,
  useCmaEventSnapshot,
  useCmaEventStream,
} from "@/hooks/use-nyxbot-cma-agents";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import {
  mergeCmaEvents,
  orderCmaMessages,
  snapshotCmaEvents,
} from "@/lib/assistant/cma-events";
import { cmaPublicText } from "@/schemas/nyxbot-cma-agent";

export function AssistantCmaAgentPage() {
  const { agentId } = useParams({ strict: false }) as { agentId: string };
  const search = useSearch({ strict: false }) as {
    response?: string;
    tab?: "desktop";
  };
  // A route/response change creates a fresh scope; no state leaks from the prior Agent.
  return (
    <CmaAgentView
      key={`${agentId}:${search.response ?? ""}`}
      agentId={agentId}
      responseId={search.response}
      initialTab={search.tab}
    />
  );
}

function CmaAgentView({
  agentId,
  responseId,
  initialTab,
}: {
  readonly agentId: string;
  readonly responseId?: string;
  readonly initialTab?: "desktop";
}) {
  const enabled = useFeature(FEATURE_FLAG.ASSISTANT_CMA_SANDBOXES);
  const [tab, setTab] = useState(initialTab ?? "execution");
  const [history, setHistory] = useState(false);
  const agent = useCmaAgent(agentId);
  const live = agent.data?.available === true;
  const task = useCmaResponse(agentId, responseId, enabled);
  const transcript = useCmaTranscript(
    agentId,
    enabled && history && tab === "execution" && live,
  );
  const snapshot = useCmaEventSnapshot(
    agentId,
    enabled && history && tab === "execution" && live,
  );
  const remote = agent.data?.cma_agent_id;
  const bound = Boolean(
    responseId && task.data?.id === responseId && task.data.agent_id === remote,
  );
  const stream = useCmaEventStream(
    agentId,
    remote,
    history ? undefined : responseId,
    enabled &&
      live &&
      tab === "execution" &&
      (history ? Boolean(snapshot.data) : bound),
    history ? snapshot.data?.pages[0]?.through_cursor : undefined,
  );
  if (!enabled)
    return <p role="status">CMA execution is not enabled for this account.</p>;
  const messages = orderCmaMessages(
    transcript.data?.pages.flatMap((page) => page.messages) ?? [],
  );
  const committed = new Set(
    messages.flatMap((message) =>
      message.sequence === null ? [] : [message.sequence],
    ),
  );
  const outputs = bound ? (task.data?.output ?? []) : [];
  const outputIds = new Set(outputs.map((item) => item.id));
  const events = mergeCmaEvents(
    history && remote
      ? snapshotCmaEvents(
          snapshot.data?.pages.flatMap((page) => page.events) ?? [],
          remote,
        )
      : [],
    stream.events,
  ).filter(
    (event) =>
      !(
        event.messageSequence !== undefined &&
        committed.has(event.messageSequence)
      ) && !(event.messageId && outputIds.has(event.messageId)),
  );
  return (
    <div className="mx-auto w-full max-w-5xl space-y-6">
      <header>
        <p className="mb-2 text-11 text-text-tertiary">CMA execution</p>
        <h1 className="text-24 font-semibold tracking-tight">
          {agent.data?.label ?? "CMA Agent"}
        </h1>
        <div className="mt-3 flex flex-wrap items-center gap-3 text-12">
          <span className="rounded-md border border-hairline px-2 py-1">
            {agent.data?.status ?? "Checking status"}
          </span>
          <span className="break-all font-mono text-text-tertiary">
            {remote}
          </span>
          {agent.data?.sandbox_id && (
            <Link
              to="/assistant/sandboxes/$sandboxId"
              params={{ sandboxId: agent.data.sandbox_id }}
              className="inline-flex items-center gap-1.5 text-text-tertiary hover:text-foreground"
            >
              <Box className="size-3" />
              Parent environment
            </Link>
          )}
        </div>
        {agent.data?.last_observed && (
          <p className="mt-2 text-11 text-text-tertiary">
            Observed {new Date(agent.data.last_observed).toLocaleString()}
          </p>
        )}
      </header>
      {agent.error && (
        <p role="alert" className="text-13 text-destructive">
          {agent.error.message}
        </p>
      )}
      {agent.data && !live && (
        <p
          role="status"
          className="rounded-lg border border-dashed border-border p-5 text-13 text-text-tertiary"
        >
          {agent.data.unavailable_reason ??
            "This Agent is currently unavailable. Its verified local reference is retained; remote history may no longer be readable."}
        </p>
      )}
      <Tabs
        value={tab}
        onValueChange={(value) => setTab(value as "desktop" | "execution")}
      >
        <TabsList>
          <TabsTrigger value="execution">
            <Activity className="mr-1.5 size-3.5" />
            Execution
          </TabsTrigger>
          <TabsTrigger value="desktop">
            <Monitor className="mr-1.5 size-3.5" />
            Desktop
          </TabsTrigger>
        </TabsList>
        <TabsContent value="execution" className="mt-5 space-y-5">
          <div className="flex flex-wrap items-center gap-2">
            <Button aria-pressed={!history} onClick={() => setHistory(false)}>
              Current task
            </Button>
            <Button aria-pressed={history} onClick={() => setHistory(true)}>
              <History />
              Agent history
            </Button>
            <Button
              variant="ghost"
              className="ml-auto"
              onClick={() => {
                void agent.refetch();
                if (history) {
                  void transcript.refetch();
                  void snapshot.refetch();
                } else if (responseId) void task.refetch();
                stream.reconnect();
              }}
            >
              <RefreshCw />
              Refresh events
            </Button>
          </div>
          {history ? (
            <p className="rounded-lg border border-hairline p-3 text-12 text-text-tertiary">
              This Agent’s history is shared across NyxBot chats that reuse it.
              These records are not exclusive to the chat where you opened this
              page.
            </p>
          ) : responseId ? (
            <div className="flex flex-wrap items-center gap-3 text-12">
              <span>Current task</span>
              <code className="break-all text-text-tertiary">{responseId}</code>
              <span>
                {bound ? task.data?.status : "Verifying task binding"}
              </span>
            </div>
          ) : (
            <p
              role="status"
              className="rounded-lg border border-dashed border-border p-6 text-13 text-text-tertiary"
            >
              Task binding is not available for this card. Open Agent history to
              inspect shared records. No latest task is selected automatically.
            </p>
          )}
          {(history ? (transcript.error ?? snapshot.error) : task.error) && (
            <p role="alert" className="text-12 text-destructive">
              {
                (history ? (transcript.error ?? snapshot.error) : task.error)
                  ?.message
              }
            </p>
          )}
          {responseId && task.data && !bound && (
            <p role="alert" className="text-12 text-destructive">
              The response could not be bound to this Agent.
            </p>
          )}
          <div className="space-y-3">
            {history
              ? messages.map((message, index) => (
                  <RecordCard
                    key={
                      message.sequence ?? message.id ?? `unsequenced-${index}`
                    }
                    title={message.role}
                    text={cmaPublicText(message.content)}
                    time={message.created_at}
                  />
                ))
              : outputs.map((item) => (
                  <RecordCard
                    key={item.id}
                    title={
                      item.type === "message"
                        ? (item.role ?? "Assistant")
                        : (item.name ?? "Tool activity")
                    }
                    text={cmaPublicText(item.content)}
                    status={item.status}
                  />
                ))}
            {events.map((event) => (
              <RecordCard
                key={event.key}
                title={event.type.replaceAll("_", " ").replaceAll(".", " · ")}
                text={event.message}
                status={event.status}
                time={event.createdAt}
              />
            ))}
          </div>
          {history && (
            <div className="flex gap-2">
              {transcript.hasNextPage && (
                <Button
                  isLoading={transcript.isFetchingNextPage}
                  onClick={() => void transcript.fetchNextPage()}
                >
                  Load more history
                </Button>
              )}
              {snapshot.hasNextPage && (
                <Button
                  isLoading={snapshot.isFetchingNextPage}
                  onClick={() => void snapshot.fetchNextPage()}
                >
                  Load more events
                </Button>
              )}
            </div>
          )}
          {live && (history || bound) && (
            <div className="flex flex-wrap items-center gap-3 text-11 text-text-tertiary">
              <span role="status">
                {stream.state === "connected"
                  ? "Following observed events"
                  : stream.state === "connecting"
                    ? "Connecting to events…"
                    : "Event connection is idle"}
              </span>
              {stream.state === "disconnected" && (
                <Button variant="outline" onClick={stream.reconnect}>
                  Reconnect events
                </Button>
              )}
              {stream.error && <span>{stream.error}</span>}
            </div>
          )}
          <p className="text-12 leading-relaxed text-text-tertiary">
            CMA publishes committed messages and tool activity. Closing or
            refreshing this view does not cancel the task. NyxBot continues
            tracking it and returns results to the original chat.
          </p>
        </TabsContent>
        <TabsContent value="desktop" className="mt-5">
          {tab === "desktop" && (
            <CmaDesktopPanel key={agentId} agentId={agentId} />
          )}
        </TabsContent>
      </Tabs>
    </div>
  );
}

function RecordCard({
  title,
  text,
  status,
  time,
}: {
  readonly title: string;
  readonly text: string;
  readonly status?: string;
  readonly time?: string | null;
}) {
  return (
    <article className="rounded-xl border border-hairline bg-card p-4 shadow-sm">
      <div className="flex flex-wrap items-center gap-2">
        <h3 className="text-12 font-medium capitalize">{title}</h3>
        {status && <span className="text-11 text-text-tertiary">{status}</span>}
        {time && (
          <time className="ml-auto text-11 text-text-tertiary" dateTime={time}>
            {new Date(time).toLocaleString()}
          </time>
        )}
      </div>
      {text && (
        <p className="mt-2 whitespace-pre-wrap break-words text-13 leading-relaxed text-muted-foreground">
          {text}
        </p>
      )}
    </article>
  );
}
