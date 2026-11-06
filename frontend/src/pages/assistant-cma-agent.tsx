import { useEffect, useState } from "react";
import { Link, useParams, useSearch } from "@tanstack/react-router";
import { Box, MessageSquare, Monitor, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { CmaDesktopPanel } from "@/components/assistant/cma-desktop-panel";
import { CmaConversation } from "@/components/assistant/cma-conversation";
import {
  CmaDisclosure,
  CmaEmptyState,
  CmaStatus,
  CmaTime,
} from "@/components/assistant/cma-execution-content";
import { useFeature } from "@/hooks/use-feature-flag";
import {
  useCmaAgent,
  useCmaResponse,
  useCmaTranscript,
  useCmaEventSnapshot,
  useCmaEventStream,
} from "@/hooks/use-nyxbot-cma-agents";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { mergeCmaEvents, snapshotCmaEvents } from "@/lib/assistant/cma-events";
import { linkedCmaTurn } from "@/lib/assistant/cma-conversation";

export function AssistantCmaAgentPage() {
  const { agentId } = useParams({ strict: false }) as { agentId: string };
  const search = useSearch({ strict: false }) as {
    response?: string;
    tab?: "desktop";
  };
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
  const [tab, setTab] = useState(initialTab ?? "conversation");
  const agent = useCmaAgent(agentId);
  const live = agent.data?.available === true;
  const observing = enabled && live && tab === "conversation";
  const task = useCmaResponse(agentId, responseId, observing);
  const transcript = useCmaTranscript(agentId, observing);
  const snapshot = useCmaEventSnapshot(agentId, observing);
  const remote = agent.data?.cma_agent_id;
  const bound = Boolean(
    responseId && task.data?.id === responseId && task.data.agent_id === remote,
  );
  const stream = useCmaEventStream(
    agentId,
    remote,
    undefined,
    observing && Boolean(snapshot.data),
    snapshot.data?.pages[0]?.through_cursor,
  );
  const snapshotEvents = remote
    ? snapshotCmaEvents(
        snapshot.data?.pages.flatMap((page) => page.events) ?? [],
        remote,
      )
    : [];
  // Search every loaded binding, including entries outside the live buffer.
  const linked = bound
    ? linkedCmaTurn([...snapshotEvents, ...stream.events], responseId)
    : undefined;
  const turns = transcript.data?.pages.flatMap((page) => page.turns) ?? [];
  const linkedTurn = turns.some((turn) => turn.turn_sequence === linked)
    ? linked
    : undefined;
  const scanMore =
    observing &&
    bound &&
    linked === undefined &&
    Boolean(snapshot.hasNextPage) &&
    (snapshot.data?.pages.length ?? 0) < 20;
  const { fetchNextPage, isFetchingNextPage, isFetchNextPageError } = snapshot;
  useEffect(() => {
    if (scanMore && !isFetchingNextPage && !isFetchNextPageError)
      void fetchNextPage();
  }, [scanMore, isFetchingNextPage, isFetchNextPageError, fetchNextPage]);
  const events = mergeCmaEvents(snapshotEvents, stream.events);
  const loading = agent.isPending || transcript.isPending;
  const locating =
    Boolean(responseId) &&
    live &&
    (task.isPending ||
      snapshot.isPending ||
      (scanMore && !isFetchNextPageError));
  const latest = turns.length
    ? Math.max(...turns.map((turn) => turn.turn_sequence))
    : undefined;
  const linkError = responseId && (task.error || (task.data && !bound));
  if (!enabled)
    return <p role="status">CMA execution is not enabled for this account.</p>;
  return (
    <div className="mx-auto w-full max-w-5xl space-y-6">
      <header>
        <p className="mb-2 text-11 text-text-tertiary">Cloud assistant</p>
        <h1 className="text-24 font-semibold tracking-tight">
          {agent.data?.label ?? "Cloud assistant"}
        </h1>
        <div className="mt-3 flex flex-wrap items-center gap-3 text-12">
          <span className="text-text-tertiary">Assistant status</span>
          <CmaStatus status={agent.data?.status ?? "checking_status"} />
          {agent.data?.sandbox_id && (
            <Link
              to="/assistant/sandboxes/$sandboxId"
              params={{ sandboxId: agent.data.sandbox_id }}
              className="inline-flex items-center gap-1.5 text-text-tertiary hover:text-foreground"
            >
              <Box className="size-3" />
              Environment
            </Link>
          )}
        </div>
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
          This assistant is currently unavailable. Its saved reference is
          retained, but its conversation and desktop may no longer be
          accessible.
        </p>
      )}
      <Tabs
        value={tab}
        onValueChange={(value) => setTab(value as "desktop" | "conversation")}
      >
        <TabsList>
          <TabsTrigger value="conversation">
            <MessageSquare className="mr-1.5 size-3.5" />
            Conversation
          </TabsTrigger>
          <TabsTrigger value="desktop">
            <Monitor className="mr-1.5 size-3.5" />
            Desktop
          </TabsTrigger>
        </TabsList>
        <TabsContent value="conversation" className="mt-5 space-y-5">
          <div className="flex flex-wrap items-center gap-3">
            <p className="min-w-0 flex-1 text-12 leading-relaxed text-text-tertiary">
              Work requests and replies from all NyxBot chats that use this
              assistant. Recent activity appears first.
            </p>
            <Button
              variant="ghost"
              onClick={() => {
                void agent.refetch();
                void transcript.refetch();
                void snapshot.refetch();
                if (responseId) void task.refetch();
                stream.reconnect();
              }}
            >
              <RefreshCw />
              Refresh
            </Button>
          </div>
          {responseId && (
            <div className="flex flex-wrap items-center gap-3 rounded-lg border border-hairline p-3 text-12">
              <p className="min-w-0 flex-1">
                {locating
                  ? "Finding the request from this link…"
                  : linkError
                    ? "This request link could not be verified for this assistant."
                    : linkedTurn !== undefined
                      ? linkedTurn === latest
                        ? "This link points to the latest request."
                        : "You are viewing an earlier request. Newer activity is available."
                      : "This request could not be located in the available conversation."}
              </p>
              <Button asChild variant="outline">
                <Link
                  to="/assistant/cma-agents/$agentId"
                  params={{ agentId }}
                  search={{}}
                >
                  View latest
                </Link>
              </Button>
            </div>
          )}
          {transcript.error && (
            <p role="alert" className="text-12 text-destructive">
              {transcript.error.message}
            </p>
          )}
          {linkError && (
            <p role="alert" className="text-12 text-destructive">
              The linked request is unavailable or does not belong to this
              assistant.
            </p>
          )}
          {snapshot.error && (
            <p role="alert" className="text-12 text-destructive">
              Execution details could not be updated. Refresh to try again.
            </p>
          )}
          {(loading || locating) && live && (
            <p
              role="status"
              className="py-6 text-center text-13 text-text-tertiary"
            >
              Loading conversation…
            </p>
          )}
          {!loading && !locating && turns.length > 0 && (
            <CmaConversation
              key={`${responseId ?? "latest"}:${linkedTurn ?? "unresolved"}`}
              turns={turns}
              events={events}
              linkedTurn={linkedTurn}
              hasLinkedRequest={Boolean(responseId)}
            />
          )}
          {!loading &&
            !locating &&
            !turns.length &&
            live &&
            !transcript.error && (
              <CmaEmptyState
                title="No conversation yet"
                text="Work requests and replies will appear here when this assistant starts working."
              />
            )}
          {transcript.hasNextPage && (
            <Button
              isLoading={transcript.isFetchingNextPage}
              onClick={() => void transcript.fetchNextPage()}
            >
              Load more conversation
            </Button>
          )}
          {live && (
            <div className="flex flex-wrap items-center gap-3 text-11 text-text-tertiary">
              <span role="status">
                {stream.state === "connected"
                  ? "Live updates connected"
                  : stream.state === "connecting"
                    ? "Connecting to live updates…"
                    : "Live updates paused"}
              </span>
              {stream.state === "disconnected" && (
                <Button variant="outline" onClick={stream.reconnect}>
                  Reconnect
                </Button>
              )}
            </div>
          )}
          <CmaDisclosure title="Assistant details">
            <dl className="space-y-2 text-11 text-text-tertiary">
              <div>
                <dt>Assistant reference</dt>
                <dd className="break-all font-mono">{remote}</dd>
              </div>
              {agent.data?.last_observed && (
                <div>
                  <dt>Last updated</dt>
                  <dd>
                    <CmaTime value={agent.data.last_observed} />
                  </dd>
                </div>
              )}
            </dl>
            {stream.error && (
              <p className="mt-3 text-12 text-text-tertiary">{stream.error}</p>
            )}
            {snapshot.hasNextPage && (
              <Button
                className="mt-3"
                isLoading={isFetchingNextPage}
                onClick={() => void fetchNextPage()}
              >
                Load more execution details
              </Button>
            )}
          </CmaDisclosure>
          <p className="text-12 leading-relaxed text-text-tertiary">
            Continue working with this assistant through NyxBot. Results return
            to the original chat.
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
