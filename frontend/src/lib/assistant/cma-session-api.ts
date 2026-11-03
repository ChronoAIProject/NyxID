import { z } from "zod";
import { ApiError, apiFetch } from "@/lib/api-client";
import { getAssistantIdentityUserId } from "./identity";
import {
  applyCmaEvent,
  CmaSseDecoder,
  cmaSessionStatus,
} from "./cma-session-events";
import {
  cmaAgentIdSchema,
  cmaAgentPageSchema,
  cmaAgentSchema,
  cmaDiscoverySchema,
  cmaDurableEventSchema,
  cmaEventHeadSchema,
  type CmaConnection,
  type CmaRoutes,
  type CmaSessionReference,
  type CmaSessionState,
} from "@/schemas/cma-session";

const DEFAULT_ROUTES: CmaRoutes = {
  agents: "/api/v1/agents",
  agent: "/api/v1/agents/{id}",
  head: "/api/v2/agents/{agent_id}/events/head",
  events: "/api/v2/agents/{agent_id}/events",
};

function guard(ownerId: string) {
  if (getAssistantIdentityUserId() !== ownerId)
    throw new DOMException("Account changed", "AbortError");
}

function request(
  ownerId: string,
  endpoint: string,
  signal: AbortSignal,
  stream = false,
) {
  return apiFetch(endpoint, {
    signal,
    authorityGuard: () => guard(ownerId),
    preserveSessionOn401: /^\/proxy\/[0-9a-f-]{36}\//i.test(endpoint),
    headers: { Accept: stream ? "text/event-stream" : "application/json" },
  });
}

async function json(
  ownerId: string,
  endpoint: string,
  signal: AbortSignal,
): Promise<unknown> {
  const response = await request(ownerId, endpoint, signal);
  const reader = response.body?.getReader();
  if (!reader) throw new Error("CMA returned an empty response.");
  const decoder = new TextDecoder();
  let text = "";
  let size = 0;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      guard(ownerId);
      if (done) break;
      size += value.length;
      if (size > 4 * 1024 * 1024)
        throw new Error("CMA returned too much session data.");
      text += decoder.decode(value, { stream: true });
    }
    return JSON.parse(text + decoder.decode()) as unknown;
  } finally {
    await reader.cancel().catch(() => undefined);
    reader.releaseLock();
  }
}

export function cmaProxyPath(
  connectionId: string,
  route: string,
  agentId?: string,
): string {
  z.uuid().parse(connectionId);
  if (!/^\/[A-Za-z0-9_{}/-]+$/.test(route) || route.includes("//"))
    throw new Error("Invalid CMA API route.");
  const path = route.replace(/\{[A-Za-z0-9_]+\}/g, () =>
    cmaAgentIdSchema.parse(agentId),
  );
  return `/proxy/${connectionId}${path}`;
}

export async function listCmaConnections(
  ownerId: string,
  signal: AbortSignal,
): Promise<CmaConnection[]> {
  const connections = new Map<string, CmaConnection>();
  for (let page = 1; page <= 100; page++) {
    const result = cmaDiscoverySchema.parse(
      await json(ownerId, `/proxy/services?page=${page}&per_page=100`, signal),
    );
    for (const service of [...result.services, ...result.custom_services]) {
      if (
        /(?:^|[^a-z0-9])cma(?:$|[^a-z0-9])/i.test(
          `${service.slug} ${service.name}`,
        ) &&
        (service.connected || !service.requires_connection)
      )
        connections.set(service.id, service);
    }
    if (result.page * result.per_page >= result.total) break;
  }
  return [...connections.values()].sort((a, b) => a.name.localeCompare(b.name));
}

export async function resolveCmaRoutes(
  ownerId: string,
  connectionId: string,
  signal: AbortSignal,
): Promise<CmaRoutes> {
  let spec: unknown;
  try {
    spec = await json(
      ownerId,
      `/proxy/services/${z.uuid().parse(connectionId)}/openapi.json`,
      signal,
    );
  } catch (error) {
    if (error instanceof ApiError && [404, 501].includes(error.status))
      return DEFAULT_ROUTES;
    throw error;
  }
  const document = z
    .object({ paths: z.record(z.string(), z.unknown()) })
    .parse(spec);
  const gets = Object.entries(document.paths)
    .filter(
      ([, value]) =>
        z.object({ get: z.object({}).passthrough() }).safeParse(value).success,
    )
    .map(([path]) => path);
  const agents = gets.find((path) => /\/agents$/.test(path));
  const agent = gets.find((path) => /\/agents\/\{[A-Za-z0-9_]+\}$/.test(path));
  const head = gets.find((path) =>
    /\/agents\/\{[A-Za-z0-9_]+\}\/events\/head$/.test(path),
  );
  const events = head?.replace(/\/head$/, "");
  if (!agents || !agent || !head || !events || !gets.includes(events)) {
    throw new Error(
      "This CMA connection does not expose Agent events v2 session streaming.",
    );
  }
  return { agents, agent, head, events };
}

export async function listCmaAgents(
  ownerId: string,
  connectionId: string,
  routes: CmaRoutes,
  signal: AbortSignal,
  cursor?: string,
) {
  const query = new URLSearchParams({ limit: "50" });
  if (cursor) query.set("cursor", cursor);
  return cmaAgentPageSchema.parse(
    await json(
      ownerId,
      `${cmaProxyPath(connectionId, routes.agents)}?${query}`,
      signal,
    ),
  );
}

export async function readCmaAgent(
  ownerId: string,
  session: Pick<CmaSessionReference, "connection" | "routes" | "agentId">,
  signal: AbortSignal,
) {
  const agent = cmaAgentSchema.parse(
    await json(
      ownerId,
      cmaProxyPath(
        session.connection.id,
        session.routes.agent,
        session.agentId,
      ),
      signal,
    ),
  );
  if (agent.id !== session.agentId)
    throw new Error("CMA returned a different session.");
  return agent;
}

export async function listActiveCmaSessions(
  ownerId: string,
  connection: CmaConnection,
  routes: CmaRoutes,
  signal: AbortSignal,
): Promise<CmaSessionReference[]> {
  const sessions = new Map<string, CmaSessionReference>();
  const cursors = new Set<string>();
  let cursor: string | undefined;
  for (let page = 0; page < 100; page++) {
    const result = await listCmaAgents(
      ownerId,
      connection.id,
      routes,
      signal,
      cursor,
    );
    for (const agent of result.items) {
      if (
        ["starting", "working", "waiting_approval", "paused"].includes(
          agent.status,
        )
      ) {
        sessions.set(agent.id, {
          connection,
          routes,
          agentId: agent.id,
          title: (agent.owner_title || agent.title || "CMA session").slice(
            0,
            300,
          ),
        });
      }
    }
    if (!result.next) return [...sessions.values()];
    if (cursors.has(result.next))
      throw new Error("CMA repeated a session list cursor.");
    cursors.add(result.next);
    cursor = result.next;
  }
  throw new Error("CMA session discovery exceeded its page limit.");
}

export function cmaSessionError(error: unknown): string {
  if (error instanceof ApiError) {
    if ([401, 403].includes(error.status))
      return "CMA access is unavailable. Check your connection.";
    if (error.status === 404)
      return "This CMA session is unavailable or you no longer have access.";
    if (error.status === 409)
      return "This CMA session changed. Reconnect to refresh its status.";
    return "CMA status is temporarily unavailable.";
  }
  if (error instanceof z.ZodError || error instanceof SyntaxError)
    return "CMA returned an unsupported session update.";
  if (error instanceof TypeError)
    return "CMA status is temporarily unavailable.";
  return error instanceof Error
    ? error.message
    : "Unable to read the CMA session.";
}

export function initialCmaSession(
  session: CmaSessionReference,
): CmaSessionState {
  return {
    connection: "connecting",
    status: "unknown",
    title: session.title,
    actions: [],
  };
}

/** Observation is independent of NyxBot turns. Reconnect only the GET tail. */
export async function watchCmaSession(
  ownerId: string,
  session: CmaSessionReference,
  signal: AbortSignal,
  update: (state: CmaSessionState) => void,
): Promise<void> {
  let state = initialCmaSession(session);
  let cursor: string | undefined;
  let incarnation: string | undefined;
  let sequence = 0;
  let failures = 0;
  const publish = (change: Partial<CmaSessionState>) => {
    if (signal.aborted || getAssistantIdentityUserId() !== ownerId) return;
    state = { ...state, ...change };
    update(state);
  };
  while (!signal.aborted) {
    const attempt = new AbortController();
    const abort = () => attempt.abort();
    signal.addEventListener("abort", abort, { once: true });
    let reader: ReadableStreamDefaultReader<Uint8Array> | undefined;
    let idle: ReturnType<typeof setTimeout> | undefined;
    try {
      guard(ownerId);
      publish({
        connection: failures ? "reconnecting" : "connecting",
        error: undefined,
      });
      idle = setTimeout(abort, 30_000);
      if (!cursor) {
        const [agent, headValue] = await Promise.all([
          readCmaAgent(ownerId, session, attempt.signal),
          json(
            ownerId,
            cmaProxyPath(
              session.connection.id,
              session.routes.head,
              session.agentId,
            ),
            attempt.signal,
          ),
        ]);
        const head = cmaEventHeadSchema.parse(headValue);
        if (head.agent_id !== session.agentId)
          throw new Error("CMA returned a different session.");
        incarnation = head.incarnation;
        cursor = head.active_turn?.cursor ?? head.through_cursor;
        sequence = head.active_turn?.sequence ?? 0;
        const status = cmaSessionStatus(agent.status);
        publish({
          title: (agent.owner_title || agent.title || session.title).slice(
            0,
            300,
          ),
          status:
            head.active_turn && ["ready", "unknown"].includes(status)
              ? "working"
              : status,
        });
      }
      const query = new URLSearchParams({
        after_cursor: cursor,
        follow: "true",
      });
      const response = await request(
        ownerId,
        `${cmaProxyPath(session.connection.id, session.routes.events, session.agentId)}?${query}`,
        attempt.signal,
        true,
      );
      if (
        !response.headers
          .get("content-type")
          ?.toLowerCase()
          .startsWith("text/event-stream") ||
        !response.body
      ) {
        throw new Error(
          "This CMA connection did not return a session status stream.",
        );
      }
      reader = response.body.getReader();
      const text = new TextDecoder();
      const sse = new CmaSseDecoder();
      publish({ connection: "live", error: undefined });
      for (;;) {
        clearTimeout(idle);
        idle = setTimeout(abort, 90_000);
        const { done, value } = await reader.read();
        guard(ownerId);
        if (signal.aborted) return;
        if (done) break;
        failures = 0;
        for (const frame of sse.push(text.decode(value, { stream: true }))) {
          if (frame.event === "cma.agent.error")
            throw new Error(
              "CMA ended session observation. Check access and reconnect.",
            );
          if (frame.event === "cma.agent.live_gap") {
            publish({
              notice:
                "Some live updates were missed. Recorded activity will catch up.",
            });
          }
          if (frame.event !== "cma.agent.event") continue;
          const event = cmaDurableEventSchema.parse(JSON.parse(frame.data));
          if (
            event.agent_id !== session.agentId ||
            event.incarnation !== incarnation ||
            frame.id !== event.cursor
          ) {
            throw new Error("CMA returned an update for a different session.");
          }
          if (event.sequence <= sequence) continue;
          const next = applyCmaEvent(state, event);
          guard(ownerId);
          if (signal.aborted) return;
          state = next;
          update(state);
          sequence = event.sequence;
          cursor = event.cursor;
        }
      }
    } catch (error) {
      if (signal.aborted || getAssistantIdentityUserId() !== ownerId) return;
      const retryable =
        error instanceof TypeError ||
        attempt.signal.aborted ||
        (error instanceof ApiError &&
          (error.status === 429 || error.status >= 500));
      publish({
        connection: retryable ? "reconnecting" : "unavailable",
        error: cmaSessionError(error),
      });
      if (!retryable) return;
    } finally {
      clearTimeout(idle);
      attempt.abort();
      signal.removeEventListener("abort", abort);
      await reader?.cancel().catch(() => undefined);
      reader?.releaseLock();
    }
    if (signal.aborted) return;
    publish({ connection: "reconnecting" });
    await reconnectDelay(
      Math.min(1000 * 2 ** Math.min(failures++, 5), 30_000),
      signal,
    );
  }
}

function reconnectDelay(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    const finish = () => {
      clearTimeout(timer);
      signal.removeEventListener("abort", finish);
      resolve();
    };
    const timer = setTimeout(finish, ms);
    signal.addEventListener("abort", finish, { once: true });
    if (signal.aborted) finish();
  });
}
