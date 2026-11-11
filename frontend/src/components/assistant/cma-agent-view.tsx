import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { useEffect, useState, type ReactNode } from "react";
import { Link } from "@tanstack/react-router";
import {
  Bot,
  Box,
  Info,
  Link2,
  MessageSquare,
  Monitor,
  RefreshCw,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { CmaDesktopPanel } from "@/components/assistant/cma-desktop-panel";
import { CmaConversation } from "@/components/assistant/cma-conversation";
import {
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
} from "@/hooks/use-assistant-cma-agents";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { mergeCmaEvents, snapshotCmaEvents } from "@/lib/assistant/cma-events";
import { linkedCmaTurn } from "@/lib/assistant/cma-conversation";

export function CmaAgentView({
  agentId,
  responseId,
  initialTab,
  presentation = "page",
  footerAction,
}: {
  readonly agentId: string;
  readonly responseId?: string;
  readonly initialTab?: "desktop";
  readonly presentation?: "page" | "panel";
  readonly footerAction?: ReactNode;
}) {
  useUiTranslation();
  const panel = presentation === "panel";
  const Heading = panel ? "h2" : "h1";
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
  const linkError = responseId && (task.error || (task.data && !bound));
  if (!enabled)
    return <p role="status">{translateUi("CMA execution is not enabled for this account.")}</p>;
  return (
    <div className="mx-auto flex h-full min-h-0 w-full max-w-5xl flex-col gap-4">
      <header className="flex shrink-0 items-start gap-3">
        <span className="mt-1 flex size-10 shrink-0 items-center justify-center rounded-xl border border-hairline bg-card text-nyx-secondary-400">
          <Bot aria-hidden className="size-5" />
        </span>
        <div className="min-w-0">
          {!panel && (
            <p className="mb-1 type-secondary text-text-tertiary">{translateUi("Cloud assistant")}</p>
          )}
          <Heading
            className={
              panel
                ? "type-dialog-title tracking-tight"
                : "type-compact-page-title sm:type-page-title tracking-tight"
            }
          >
            {agent.data?.label ?? translateUi("Cloud assistant")}
          </Heading>
          <div className="mt-2 flex flex-wrap items-center gap-3 text-12">
            <span className="sr-only">{translateUi("Assistant status")}</span>
            <CmaStatus status={agent.data?.status ?? "checking_status"} />
            {agent.data?.sandbox_id && (
              <Link
                to="/assistant/sandboxes/$sandboxId"
                params={{ sandboxId: agent.data.sandbox_id }}
                target={panel ? "_blank" : undefined}
                rel={panel ? "noopener noreferrer" : undefined}
                className="inline-flex items-center gap-1.5 text-text-tertiary hover:text-foreground"
              >
                <Box className="size-3" />{translateUi("Environment")}</Link>
            )}
          </div>
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
        >{translateUi("This assistant is currently unavailable. Its saved reference is retained, but its conversation and desktop may no longer be accessible.")}</p>
      )}
      <Tabs
        value={tab}
        onValueChange={(value) => setTab(value as "desktop" | "conversation")}
        className="flex min-h-0 flex-1 flex-col"
      >
        <div className="flex shrink-0 items-center justify-between gap-3">
          <TabsList>
            <TabsTrigger value="conversation">
              <MessageSquare className="mr-1.5 size-3.5" />{translateUi("Conversation")}</TabsTrigger>
            <TabsTrigger value="desktop">
              <Monitor className="mr-1.5 size-3.5" />{translateUi("Desktop")}</TabsTrigger>
          </TabsList>
          {tab === "conversation" && (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => {
                void agent.refetch();
                void transcript.refetch();
                void snapshot.refetch();
                if (responseId) void task.refetch();
                stream.reconnect();
              }}
            >
              <RefreshCw />{translateUi("Refresh")}</Button>
          )}
        </div>
        <TabsContent
          forceMount
          value="conversation"
          className="mt-4 flex min-h-0 flex-1 flex-col gap-3 data-[state=inactive]:hidden"
        >
          {responseId && (
            <div className="flex shrink-0 items-center gap-2 border-b border-hairline pb-3 type-secondary text-text-tertiary">
              <Link2 aria-hidden className="size-3 shrink-0" />
              <p className="min-w-0 flex-1">
                {locating
                  ? translateUi("Finding the request from this link…")
                  : linkError
                    ? translateUi("This request link could not be verified for this assistant.")
                    : linkedTurn !== undefined
                      ? translateUi("Opened at the request from your chat. Earlier and newer messages remain in the conversation.")
                      : translateUi("This request could not be located in the available conversation.")}
              </p>
            </div>
          )}
          {transcript.error && (
            <p role="alert" className="text-12 text-destructive">
              {transcript.error.message}
            </p>
          )}
          {linkError && (
            <p role="alert" className="text-12 text-destructive">{translateUi("The linked request is unavailable or does not belong to this assistant.")}</p>
          )}
          {snapshot.error && (
            <p role="alert" className="text-12 text-destructive">{translateUi("Execution details could not be updated. Refresh to try again.")}</p>
          )}
          {(loading || locating) && live && (
            <p
              role="status"
              className="py-6 text-center text-13 text-text-tertiary"
            >{translateUi("Loading conversation…")}</p>
          )}
          {!loading && turns.length > 0 && (
            <CmaConversation
              key={responseId ?? "latest"}
              turns={turns}
              events={events}
              linkedTurn={linkedTurn}
              hasLinkedRequest={Boolean(responseId)}
              assistantName={agent.data?.label}
              active={tab === "conversation"}
              historyControl={
                transcript.hasNextPage ? (
                  <Button
                    isLoading={transcript.isFetchingNextPage}
                    onClick={() => void transcript.fetchNextPage()}
                  >{translateUi("Load earlier conversation")}</Button>
                ) : undefined
              }
            />
          )}
          {!loading &&
            !locating &&
            !turns.length &&
            live &&
            !transcript.error && (
              <CmaEmptyState
                title={translateUi("No conversation yet")}
                text="Work requests and replies will appear here when this assistant starts working."
              />
            )}
        </TabsContent>
        <TabsContent
          value="desktop"
          className="mt-5 min-h-0 flex-1 overflow-y-auto"
        >
          {tab === "desktop" && (
            <CmaDesktopPanel key={agentId} agentId={agentId} />
          )}
        </TabsContent>
      </Tabs>
      <footer className="flex shrink-0 flex-wrap items-center gap-3 border-t border-hairline pt-3">
        <p className="min-w-0 basis-full type-secondary text-text-tertiary sm:min-w-32 sm:flex-1 sm:basis-auto">
          {panel
            ? translateUi("Continue through NyxBot")
            : translateUi("Continue working with this assistant through NyxBot. Results return to the original chat.")}
        </p>
        {live && tab === "conversation" && (
          <div className="flex min-w-0 flex-1 flex-wrap items-center gap-3 type-secondary text-text-tertiary sm:flex-none">
            <span role="status">
              {stream.state === "connected"
                ? translateUi("Live updates connected")
                : stream.state === "connecting"
                  ? translateUi("Connecting to live updates…")
                  : translateUi("Live updates paused")}
            </span>
            {stream.state === "disconnected" && (
              <Button variant="outline" onClick={stream.reconnect}>{translateUi("Reconnect")}</Button>
            )}
          </div>
        )}
        {footerAction}
        <Dialog>
          <DialogTrigger asChild>
            <Button variant="ghost" size="icon" aria-label={translateUi("Assistant details")}>
              <Info aria-hidden />
            </Button>
          </DialogTrigger>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>{translateUi("Assistant details")}</DialogTitle>
              <DialogDescription>{translateUi("Saved reference and live connection information for this assistant.")}</DialogDescription>
            </DialogHeader>
            <dl className="space-y-2 type-secondary text-text-tertiary">
              <div>
                <dt>{translateUi("Assistant reference")}</dt>
                <dd className="break-all font-mono">{remote}</dd>
              </div>
              {agent.data?.last_observed && (
                <div>
                  <dt>{translateUi("Last updated")}</dt>
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
              >{translateUi("Load more execution details")}</Button>
            )}
          </DialogContent>
        </Dialog>
      </footer>
    </div>
  );
}
