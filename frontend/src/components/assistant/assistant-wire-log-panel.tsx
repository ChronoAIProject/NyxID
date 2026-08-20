import { useEffect, useState } from "react";
import { ChevronDown, Copy, Network, Trash2 } from "lucide-react";
import { AssistantWireReplayView } from "@/components/assistant/assistant-wire-replay-view";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
  SheetTrigger,
} from "@/components/ui/sheet";
import { Switch } from "@/components/ui/switch";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import type {
  AssistantUpstreamEnvelope,
  AssistantWireLogExchange,
} from "@/schemas/assistant-wire-log";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { useAssistantWireLogStore } from "@/stores/assistant-wire-log-store";
import { cn } from "@/lib/utils";

const INITIAL_RAW_LINE_WINDOW = 200;
const RAW_LINE_WINDOW_STEP = 200;

function statusVariant(status: number): "success" | "destructive" | "warning" {
  if (status >= 200 && status < 400) return "success";
  if (status >= 500) return "destructive";
  return "warning";
}

function normalizedPath(path: string): string {
  return path.startsWith("/") ? path : `/${path}`;
}

function requestJson(envelope: AssistantUpstreamEnvelope): string {
  if (envelope.degraded) return JSON.stringify(envelope, null, 2);
  return JSON.stringify(
    {
      method: envelope.method,
      path: normalizedPath(envelope.path),
      commandType: envelope.commandType,
      body: envelope.body,
      headers: envelope.headers,
      identity: envelope.identity,
      truncated: envelope.truncated,
      droppedBody: envelope.droppedBody,
      droppedHeaders: envelope.droppedHeaders,
    },
    null,
    2,
  );
}

function entryJson(entry: AssistantWireLogExchange): string {
  return JSON.stringify(entry, null, 2);
}

function upstreamStatus(envelope: AssistantUpstreamEnvelope): number | null {
  if (envelope.degraded) return envelope.status ?? null;
  return envelope.response?.status ?? null;
}

function UpstreamResponse({
  envelope,
  index,
}: {
  readonly envelope: AssistantUpstreamEnvelope;
  readonly index: number;
}) {
  const outcome = envelope.upstreamOutcome ?? "unknown";
  const response = envelope.degraded ? null : envelope.response;
  return (
    <section className="space-y-2">
      <div className="flex flex-wrap items-center gap-2">
        <h4 className="text-[10px] font-semibold uppercase text-text-tertiary">
          Upstream response {index + 1}
        </h4>
        {response ? (
          <Badge variant={statusVariant(response.status)}>
            Aevatar {response.status}
          </Badge>
        ) : null}
      </div>
      {outcome === "no_response" ? (
        <p className="text-[11px] text-warning">No upstream response.</p>
      ) : outcome === "unknown" ? (
        <p className="text-[11px] text-text-tertiary">
          Response metadata not reported by this backend.
        </p>
      ) : response ? (
        <div className="rounded-lg border border-border/60 bg-background/50 p-3 text-[11px]">
          <p className="text-muted-foreground">
            Backend-observed Aevatar response metadata
          </p>
          <dl className="mt-2 grid grid-cols-[max-content_1fr] gap-x-3 gap-y-1 font-mono text-[10px]">
            <dt className="text-text-tertiary">status</dt>
            <dd>{response.status}</dd>
            <dt className="text-text-tertiary">SSE</dt>
            <dd>{response.sse ? "yes" : "no"}</dd>
            {Object.entries(response.headers).map(([name, header]) => (
              <div key={name} className="contents">
                <dt className="break-all text-text-tertiary">{name}</dt>
                <dd className="min-w-0 break-all">
                  {header.value}
                  {header.truncated ? " [truncated]" : ""}
                </dd>
              </div>
            ))}
          </dl>
          {!envelope.degraded && envelope.droppedHeaders ? (
            <p className="mt-2 text-[10px] text-warning">
              Allowlisted response headers were dropped by the wire-header size
              ladder.
            </p>
          ) : null}
        </div>
      ) : envelope.degraded && envelope.status ? (
        <p className="text-[11px] text-muted-foreground">
          Minimal backend echo retained status {envelope.status}; response
          headers were degraded.
        </p>
      ) : (
        <p className="text-[11px] text-text-tertiary">
          Backend reported a response, but its metadata was degraded.
        </p>
      )}
    </section>
  );
}

function ResponseCapture({
  entry,
  rendered,
  onRenderedChange,
  visibleLines,
  onShowMore,
}: {
  readonly entry: AssistantWireLogExchange;
  readonly rendered: boolean;
  readonly onRenderedChange: (rendered: boolean) => void;
  readonly visibleLines: number;
  readonly onShowMore: () => void;
}) {
  const capture = entry.capture;
  if (!capture) {
    return (
      <p className="text-[11px] text-text-tertiary">
        Delivered-body capture is not available for this persisted exchange.
      </p>
    );
  }
  if (capture.state === "evicted") {
    return (
      <p className="text-[11px] text-text-tertiary">
        Delivered-body capture was evicted from session memory.
      </p>
    );
  }
  const hasSse = Boolean(capture.sse);
  const truncated = capture.sse?.truncated ?? capture.body?.truncated ?? false;
  return (
    <section className="space-y-2">
      <div className="flex flex-wrap items-center gap-2">
        <h4 className="text-[10px] font-semibold uppercase text-text-tertiary">
          Delivered response
        </h4>
        <Badge variant="secondary">
          {capture.state === "settled" ? capture.outcome : "capturing"}
        </Badge>
        {truncated ? <Badge variant="warning">Truncated</Badge> : null}
        {hasSse ? (
          <div
            role="tablist"
            aria-label="Delivered response view"
            className="ml-auto flex h-7 items-center rounded-lg border border-border/60 p-0.5"
          >
            <button
              type="button"
              role="tab"
              aria-selected={!rendered}
              onClick={() => onRenderedChange(false)}
              className={cn(
                "h-6 rounded-md px-2 text-[10px] font-medium",
                !rendered
                  ? "bg-white/[0.08] text-foreground"
                  : "text-text-tertiary hover:text-muted-foreground",
              )}
            >
              Raw
            </button>
            <button
              type="button"
              role="tab"
              aria-selected={rendered}
              onClick={() => onRenderedChange(true)}
              className={cn(
                "h-6 rounded-md px-2 text-[10px] font-medium",
                rendered
                  ? "bg-white/[0.08] text-foreground"
                  : "text-text-tertiary hover:text-muted-foreground",
              )}
            >
              Rendered
            </button>
          </div>
        ) : null}
      </div>
      <p className="text-[11px] text-muted-foreground">
        Decoded response entity as delivered by NyxID to this browser.
      </p>
      {capture.sse && rendered ? (
        <AssistantWireReplayView exchange={entry} />
      ) : capture.sse ? (
        <div className="overflow-hidden rounded-lg border border-border/60 bg-background/60">
          <div className="max-h-96 overflow-auto font-mono text-[10px] leading-5">
            {capture.sse.lines.slice(0, visibleLines).map((line, index) => (
              <div
                key={`${String(index)}-${line.ending}`}
                title={`Line ending: ${line.ending || "unterminated"}`}
                className="grid min-h-5 grid-cols-[3rem_1fr] border-b border-border/30 last:border-0"
              >
                <span className="select-none border-r border-border/30 px-2 text-right text-text-tertiary">
                  {index + 1}
                </span>
                <span className="min-w-0 whitespace-pre-wrap break-all px-2 text-muted-foreground">
                  {line.text}
                </span>
              </div>
            ))}
          </div>
          {visibleLines < capture.sse.lines.length ? (
            <div className="border-t border-border/60 p-2 text-center">
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={onShowMore}
              >
                Show{" "}
                {Math.min(
                  RAW_LINE_WINDOW_STEP,
                  capture.sse.lines.length - visibleLines,
                )}{" "}
                more
              </Button>
            </div>
          ) : null}
        </div>
      ) : capture.body ? (
        <pre className="max-h-80 overflow-auto whitespace-pre-wrap break-all rounded-lg border border-border/60 bg-background/60 p-3 font-mono text-[10px] leading-5 text-muted-foreground">
          {capture.body.text}
        </pre>
      ) : (
        <p className="text-[11px] text-text-tertiary">
          Waiting for delivered response data.
        </p>
      )}
    </section>
  );
}

export function AssistantWireLogPanel() {
  const entries = useAssistantWireLogStore((state) => state.entries);
  const captureEnabled = useAssistantWireLogStore(
    (state) => state.captureEnabled,
  );
  const showResponses = useAssistantWireLogStore(
    (state) => state.showResponses,
  );
  const setCaptureEnabled = useAssistantWireLogStore(
    (state) => state.setCaptureEnabled,
  );
  const setShowResponses = useAssistantWireLogStore(
    (state) => state.setShowResponses,
  );
  const clear = useAssistantWireLogStore((state) => state.clear);
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(new Set());
  const [rendered, setRendered] = useState<ReadonlySet<string>>(new Set());
  const [lineWindows, setLineWindows] = useState<
    Readonly<Record<string, number>>
  >({});

  function toggleSet(
    setter: React.Dispatch<React.SetStateAction<ReadonlySet<string>>>,
    id: string,
    value?: boolean,
  ) {
    setter((current) => {
      const next = new Set(current);
      const shouldAdd = value ?? !next.has(id);
      if (shouldAdd) next.add(id);
      else next.delete(id);
      return next;
    });
  }

  return (
    <Sheet>
      <Tooltip>
        <TooltipTrigger asChild>
          <SheetTrigger asChild>
            <Button
              type="button"
              variant="ghost"
              size="icon"
              aria-label="Aevatar wire log"
              className="shrink-0 text-text-tertiary"
            >
              <Network className="h-4 w-4" />
            </Button>
          </SheetTrigger>
        </TooltipTrigger>
        <TooltipContent>Aevatar wire log</TooltipContent>
      </Tooltip>

      <SheetContent
        side="right"
        className="flex w-full flex-col gap-0 p-0 sm:max-w-2xl"
      >
        <SheetHeader className="shrink-0 border-b border-border/60 px-5 py-5 pr-12">
          <SheetTitle>Aevatar wire log</SheetTitle>
          <SheetDescription className="sr-only">
            Backend-sourced Aevatar request echoes, response metadata, and
            response entities delivered by NyxID.
          </SheetDescription>
        </SheetHeader>

        <div className="flex shrink-0 flex-wrap items-center gap-x-5 gap-y-2 border-b border-border/60 px-5 py-3">
          <label
            htmlFor="wire-log-capture"
            className="flex items-center gap-2 text-[12px] font-medium text-foreground"
          >
            <Switch
              id="wire-log-capture"
              checked={captureEnabled}
              onCheckedChange={setCaptureEnabled}
              aria-label="Capture Aevatar exchanges"
            />
            Capture
          </label>
          <label
            htmlFor="wire-log-responses"
            className="flex items-center gap-2 text-[12px] font-medium text-foreground"
          >
            <Switch
              id="wire-log-responses"
              checked={showResponses}
              onCheckedChange={setShowResponses}
              aria-label="Show Aevatar responses"
            />
            Responses
          </label>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="ml-auto"
            onClick={clear}
            disabled={entries.length === 0}
          >
            <Trash2 />
            Clear
          </Button>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto px-3 py-3">
          {entries.length === 0 ? (
            <div className="flex min-h-40 items-center justify-center px-6 text-center text-[12px] text-text-tertiary">
              No captured requests
            </div>
          ) : (
            <div className="space-y-2">
              {[...entries].reverse().map((entry) => {
                const isExpanded = expanded.has(entry.id);
                const upstreamEchoes = entry.upstreamEchoes ?? [];
                const primary = upstreamEchoes[0];
                if (!primary) return null;
                const aevatarStatus = upstreamStatus(primary);
                return (
                  <article
                    key={entry.id}
                    className="overflow-hidden rounded-lg border border-border/70 bg-surface"
                  >
                    <div className="flex items-start gap-2 p-3">
                      <button
                        type="button"
                        onClick={() => toggleSet(setExpanded, entry.id)}
                        aria-expanded={isExpanded}
                        className="flex min-w-0 flex-1 items-start gap-2 text-left focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                      >
                        <ChevronDown
                          className={cn(
                            "mt-0.5 h-3.5 w-3.5 shrink-0 text-text-tertiary transition-transform",
                            isExpanded && "rotate-180",
                          )}
                        />
                        <span className="min-w-0 flex-1">
                          <span className="flex flex-wrap items-center gap-1.5">
                            <span className="text-[10px] tabular-nums text-text-tertiary">
                              {new Date(entry.ts).toLocaleTimeString([], {
                                hour: "2-digit",
                                minute: "2-digit",
                                second: "2-digit",
                              })}
                            </span>
                            {primary.commandType ? (
                              <Badge variant="secondary">
                                {primary.commandType}
                              </Badge>
                            ) : null}
                            <Tooltip>
                              <TooltipTrigger asChild>
                                <Badge variant={statusVariant(entry.status)}>
                                  NyxID {entry.status}
                                </Badge>
                              </TooltipTrigger>
                              <TooltipContent>
                                NyxID response to this browser
                              </TooltipContent>
                            </Tooltip>
                            {showResponses && aevatarStatus ? (
                              <Tooltip>
                                <TooltipTrigger asChild>
                                  <Badge variant={statusVariant(aevatarStatus)}>
                                    Aevatar {aevatarStatus}
                                  </Badge>
                                </TooltipTrigger>
                                <TooltipContent>
                                  Backend-observed Aevatar status
                                </TooltipContent>
                              </Tooltip>
                            ) : null}
                          </span>
                          <span className="mt-1 block break-all font-mono text-[11px] leading-5 text-muted-foreground">
                            <strong className="font-semibold text-foreground">
                              {primary.method}
                            </strong>{" "}
                            {normalizedPath(primary.path)}
                          </span>
                        </span>
                      </button>
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <Button
                            type="button"
                            variant="ghost"
                            size="icon"
                            aria-label={`Copy ${primary.method} ${primary.path} exchange as JSON`}
                            className="h-7 w-7 shrink-0"
                            onClick={() =>
                              void navigator.clipboard.writeText(
                                entryJson(entry),
                              )
                            }
                          >
                            <Copy />
                          </Button>
                        </TooltipTrigger>
                        <TooltipContent>Copy exchange as JSON</TooltipContent>
                      </Tooltip>
                    </div>
                    {isExpanded ? (
                      <div className="space-y-4 border-t border-border/60 bg-background/20 p-3">
                        {upstreamEchoes.map((envelope, index) => (
                          <section
                            key={`${envelope.path}-${String(index)}`}
                            className="space-y-2"
                          >
                            <h4 className="text-[10px] font-semibold uppercase text-text-tertiary">
                              Request {index + 1} - backend echo
                            </h4>
                            <p className="text-[11px] leading-5 text-muted-foreground">
                              Reconstructed request assembled by NyxID before
                              credential and identity injection; headers are an
                              allowlisted subset.
                            </p>
                            <pre className="max-h-80 overflow-auto whitespace-pre-wrap break-all rounded-lg border border-border/60 bg-background/60 p-3 font-mono text-[10px] leading-5 text-muted-foreground">
                              {requestJson(envelope)}
                            </pre>
                          </section>
                        ))}
                        {entry.droppedEchoCount ? (
                          <p className="text-[11px] text-warning">
                            {entry.droppedEchoCount} later backend echo
                            {entry.droppedEchoCount === 1 ? " was" : "es were"}{" "}
                            dropped by the header size ladder.
                          </p>
                        ) : null}
                        {showResponses ? (
                          <>
                            {upstreamEchoes.map((envelope, index) => (
                              <UpstreamResponse
                                key={`${envelope.path}-response-${String(index)}`}
                                envelope={envelope}
                                index={index}
                              />
                            ))}
                            <ResponseCapture
                              entry={entry}
                              rendered={rendered.has(entry.id)}
                              onRenderedChange={(value) =>
                                toggleSet(setRendered, entry.id, value)
                              }
                              visibleLines={
                                lineWindows[entry.id] ?? INITIAL_RAW_LINE_WINDOW
                              }
                              onShowMore={() =>
                                setLineWindows((current) => ({
                                  ...current,
                                  [entry.id]:
                                    (current[entry.id] ??
                                      INITIAL_RAW_LINE_WINDOW) +
                                    RAW_LINE_WINDOW_STEP,
                                }))
                              }
                            />
                          </>
                        ) : null}
                      </div>
                    ) : null}
                  </article>
                );
              })}
            </div>
          )}
        </div>

        <div className="shrink-0 space-y-1 border-t border-border/60 px-5 py-3 text-[11px] leading-4 text-text-tertiary">
          <p>
            Raw captures are session-only and may contain sensitive payloads
            verbatim without the chat renderer's credential redaction,
            including in replay placeholders.
          </p>
          <p>WebSocket exchanges are not captured.</p>
          <p>
            Final AppError responses do not carry debug echoes, including errors
            after an upstream response was observed.
          </p>
        </div>
      </SheetContent>
    </Sheet>
  );
}

export function AssistantWireLogAction() {
  // Runtime feature flag (`experimental:aevatar-chat-wire-log`), resolved
  // server-side for this user on the personal surface. Fail-closed: unknown,
  // loading, or an older backend that omits the key all read as off.
  const featureEnabled = useFeature(FEATURE_FLAG.AEVATAR_CHAT_WIRE_LOG);
  const setFeatureEnabled = useAssistantWireLogStore(
    (state) => state.setFeatureEnabled,
  );

  useEffect(() => {
    setFeatureEnabled(featureEnabled);
  }, [featureEnabled, setFeatureEnabled]);

  if (!featureEnabled) return null;
  return <AssistantWireLogPanel />;
}
