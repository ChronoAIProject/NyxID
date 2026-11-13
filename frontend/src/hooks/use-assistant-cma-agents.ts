import { useEffect, useRef, useState } from "react";
import {
  useInfiniteQuery,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { assistantHttp } from "@/lib/assistant/assistant-http";
import {
  cmaAgentSchema,
  cmaDesktopSchema,
  cmaEventSnapshotSchema,
  cmaResponseSchema,
  cmaTranscriptSchema,
} from "@/schemas/assistant-cma-agent";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { useAuthStore } from "@/stores/auth-store";
import { useFeature } from "./use-feature-flag";
import { cmaQueryKeys } from "./use-assistant-cma-sandboxes";
import {
  mergeCmaEvents,
  normalizeCmaEvent,
  readCmaStream,
  type CmaDisplayEvent,
} from "@/lib/assistant/cma-events";

import { cmaJson } from "@/lib/assistant/cma-http";

const BASE = "/assistant/nyxagent/cma-agents";
export function cmaAgentPath(id: string) {
  return `${BASE}/${encodeURIComponent(id)}`;
}
function useCmaReadScope() {
  const owner = useAuthStore((state) => state.user?.id);
  const enabled = useFeature(FEATURE_FLAG.ASSISTANT_CMA_SANDBOXES);
  return { owner, enabled: Boolean(owner) && enabled };
}
export function useCmaAgent(id: string) {
  const scope = useCmaReadScope();
  return useQuery({
    queryKey: [...cmaQueryKeys.root(scope.owner), "agent", id],
    queryFn: async ({ signal }) =>
      cmaAgentSchema.parse(await cmaJson(cmaAgentPath(id), { signal })),
    enabled: scope.enabled && Boolean(id),
    retry: false,
    refetchInterval: 10_000,
  });
}
export function useCmaResponse(
  id: string,
  responseId: string | undefined,
  enabled: boolean,
) {
  const scope = useCmaReadScope();
  return useQuery({
    queryKey: [...cmaQueryKeys.root(scope.owner), "response", id, responseId],
    queryFn: async ({ signal }) => {
      const response = cmaResponseSchema.parse(
        await cmaJson(
          `${cmaAgentPath(id)}/responses/${encodeURIComponent(responseId!)}`,
          { signal },
        ),
      );
      if (response.id !== responseId)
        throw new Error("CMA response binding could not be verified.");
      return response;
    },
    enabled: scope.enabled && enabled && Boolean(id && responseId),
    retry: false,
    refetchInterval: (query) =>
      query.state.data &&
      ["completed", "failed", "incomplete"].includes(query.state.data.status)
        ? false
        : 5_000,
  });
}
export function useCmaTranscript(id: string, enabled: boolean) {
  const scope = useCmaReadScope();
  return useInfiniteQuery({
    queryKey: [...cmaQueryKeys.root(scope.owner), "transcript", id],
    initialPageParam: null as string | null,
    queryFn: async ({ pageParam, signal }) =>
      cmaTranscriptSchema.parse(
        await cmaJson(
          `${cmaAgentPath(id)}/transcript${pageParam ? `?cursor=${encodeURIComponent(pageParam)}` : ""}`,
          { signal },
        ),
      ),
    getNextPageParam: (last, _pages, previous, pageParams) =>
      last.next_cursor &&
      last.next_cursor !== previous &&
      !pageParams.includes(last.next_cursor)
        ? last.next_cursor
        : undefined,
    enabled: scope.enabled && enabled && Boolean(id),
    retry: false,
    maxPages: 20,
  });
}
export function useCmaEventSnapshot(id: string, enabled: boolean) {
  const scope = useCmaReadScope();
  return useInfiniteQuery({
    queryKey: [...cmaQueryKeys.root(scope.owner), "events", id],
    initialPageParam: null as string | null,
    queryFn: async ({ pageParam, signal }) =>
      cmaEventSnapshotSchema.parse(
        await cmaJson(
          `${cmaAgentPath(id)}/events/snapshot${pageParam ? `?cursor=${encodeURIComponent(pageParam)}` : ""}`,
          { signal },
        ),
      ),
    getNextPageParam: (last, _pages, previous, pageParams) =>
      last.next_cursor &&
      last.next_cursor !== previous &&
      !pageParams.includes(last.next_cursor)
        ? last.next_cursor
        : undefined,
    enabled: scope.enabled && enabled && Boolean(id),
    retry: false,
    maxPages: 20,
  });
}
export function useCmaDesktop(id: string, enabled = true) {
  const scope = useCmaReadScope();
  return useQuery({
    queryKey: [...cmaQueryKeys.root(scope.owner), "desktop", id],
    queryFn: async ({ signal }) =>
      cmaDesktopSchema.parse(
        await cmaJson(`${cmaAgentPath(id)}/desktop`, { signal }),
      ),
    enabled: scope.enabled && enabled && Boolean(id),
    retry: false,
    refetchInterval: 15_000,
  });
}

type StreamState = "idle" | "connecting" | "connected" | "disconnected";
export function useCmaEventStream(
  id: string,
  remoteAgentId: string | undefined,
  responseId: string | undefined,
  enabled: boolean,
  initialCursor?: string | null,
) {
  const scope = useCmaReadScope();
  const client = useQueryClient();
  const identity = `${scope.owner}:${id}:${responseId ?? "history"}`;
  const [attempt, setAttempt] = useState(0);
  const resume = useRef<{ identity: string; cursor?: string }>({ identity });
  const [result, setResult] = useState<{
    identity: string;
    events: CmaDisplayEvent[];
    state: StreamState;
    error?: string;
  }>({ identity, events: [], state: "idle" });
  useEffect(() => {
    if (!scope.enabled || !enabled || !remoteAgentId) return;
    const abort = new AbortController();
    let disposed = false;
    let timeout: ReturnType<typeof setTimeout>;
    const current = () =>
      !disposed &&
      !abort.signal.aborted &&
      useAuthStore.getState().user?.id === scope.owner;
    const expire = () => {
      if (disposed || useAuthStore.getState().user?.id !== scope.owner) return;
      setResult((previous) => ({
        ...previous,
        identity,
        state: "disconnected",
        error:
          "CMA observation timed out. Reconnect to continue from the last cursor.",
      }));
      abort.abort();
    };
    const armIdle = () => {
      if (!current()) return;
      clearTimeout(timeout);
      timeout = setTimeout(expire, 60_000);
    };
    timeout = setTimeout(expire, 20_000);
    const lifetime = setTimeout(expire, 5 * 60_000);
    let cursor =
      resume.current.identity === identity
        ? (resume.current.cursor ?? initialCursor ?? undefined)
        : (initialCursor ?? undefined);
    resume.current = { identity, cursor };
    let refreshAt = -Infinity;
    let refreshTimer: ReturnType<typeof setTimeout> | undefined;
    const refresh = () => {
      clearTimeout(refreshTimer);
      refreshTimer = undefined;
      if (!current()) return;
      refreshAt = Date.now();
      void client.invalidateQueries({
        queryKey: [
          ...cmaQueryKeys.root(scope.owner),
          responseId ? "response" : "transcript",
          id,
        ],
      });
    };
    async function observe() {
      await Promise.resolve();
      if (!current()) return;
      setResult((previous) => ({
        identity,
        events: previous.identity === identity ? previous.events : [],
        state: "connecting",
      }));
      try {
        const path = responseId
          ? `${cmaAgentPath(id)}/responses/${encodeURIComponent(responseId)}/events`
          : `${cmaAgentPath(id)}/events`;
        // The server maps each stream's cursor onto its native CMA protocol.
        const response = await assistantHttp(
          `${path}${cursor ? `?after_cursor=${encodeURIComponent(cursor)}` : ""}`,
          {
            signal: abort.signal,
            headers: { Accept: "text/event-stream" },
            reportErrors: false,
          },
        );
        if (!current()) return;
        armIdle();
        setResult((previous) => ({ ...previous, state: "connected" }));
        for await (const frame of readCmaStream(response, armIdle)) {
          if (!current()) break;
          const data = frame.data as Record<string, unknown> | null;
          if (frame.event?.endsWith("error") || data?.type === "error")
            throw new Error(
              "CMA observation ended. Reconnect with current authorization.",
            );
          const event = normalizeCmaEvent(
            frame.data,
            { agentId: remoteAgentId!, responseId },
            frame.id,
          );
          if (frame.id) {
            cursor = frame.id;
            resume.current = { identity, cursor };
          }
          if (!event) continue;
          setResult((previous) => ({
            identity,
            state: "connected",
            events: mergeCmaEvents(
              previous.identity === identity ? previous.events : [],
              [event],
            ),
          }));
          const elapsed = Date.now() - refreshAt;
          if (elapsed >= 1_000) refresh();
          // Keep the final event of a burst: otherwise a completed turn can
          // remain active in the transcript until the next unrelated event.
          else if (refreshTimer === undefined)
            refreshTimer = setTimeout(refresh, 1_000 - elapsed);
        }
        if (current())
          setResult((previous) => ({ ...previous, state: "disconnected" }));
      } catch (error) {
        if (current())
          setResult((previous) => ({
            ...previous,
            state: "disconnected",
            error:
              error instanceof Error
                ? error.message
                : "CMA observation is unavailable.",
          }));
      } finally {
        if (refreshTimer !== undefined) refresh();
        clearTimeout(timeout);
        clearTimeout(lifetime);
      }
    }
    void observe();
    return () => {
      disposed = true;
      clearTimeout(refreshTimer);
      clearTimeout(timeout);
      clearTimeout(lifetime);
      abort.abort();
    };
  }, [
    scope.owner,
    scope.enabled,
    enabled,
    id,
    remoteAgentId,
    responseId,
    initialCursor,
    attempt,
    identity,
    client,
  ]);
  return {
    events: result.identity === identity ? result.events : [],
    state:
      result.identity === identity && enabled
        ? result.state
        : ("idle" as StreamState),
    error: result.identity === identity ? result.error : undefined,
    reconnect: () => setAttempt((value) => value + 1),
  };
}
