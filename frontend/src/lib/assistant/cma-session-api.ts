import { z } from "zod";
import { ApiError, apiFetch } from "@/lib/api-client";
import {
  getAssistantIdentityUserId,
  getAssistantIdentityRevision,
} from "./identity";
import {
  applyCmaEvent,
  CmaSseDecoder,
  cmaSessionStatus,
} from "./cma-session-events";
import {
  cmaOriginPageSchema,
  type CmaSessionOrigin,
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

class CmaAccountAuthorizationError extends ApiError {}

const DEFAULT_ROUTES: CmaRoutes = {
  agents: "/api/v1/agents",
  agent: "/api/v1/agents/{id}",
  head: "/api/v2/agents/{agent_id}/events/head",
  events: "/api/v2/agents/{agent_id}/events",
};

function guard(ownerId: string, revision = getAssistantIdentityRevision()) {
  if (
    getAssistantIdentityUserId() !== ownerId ||
    getAssistantIdentityRevision() !== revision
  )
    throw new DOMException("Account changed", "AbortError");
}

function request(
  ownerId: string,
  endpoint: string,
  signal: AbortSignal,
  stream = false,
) {
  const revision = getAssistantIdentityRevision();
  return apiFetch(endpoint, {
    signal,
    authorityGuard: () => guard(ownerId, revision),
    preserveSessionOn401: /^\/proxy\/[0-9a-f-]{36}\//i.test(endpoint),
    headers: { Accept: stream ? "text/event-stream" : "application/json" },
  });
}

async function json(
  ownerId: string,
  endpoint: string,
  signal: AbortSignal,
): Promise<unknown> {
  const revision = getAssistantIdentityRevision();
  const response = await request(ownerId, endpoint, signal).catch(
    (error: unknown) => {
      if (
        endpoint.startsWith("/assistant/") &&
        error instanceof ApiError &&
        error.status === 401
      )
        throw new CmaAccountAuthorizationError(
          error.status,
          error.errorResponse,
        );
      throw error;
    },
  );
  const reader = response.body?.getReader();
  if (!reader) throw new Error("CMA returned an empty response.");
  const decoder = new TextDecoder();
  let text = "";
  let size = 0;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      guard(ownerId, revision);
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
  status?: string,
) {
  const query = new URLSearchParams({ limit: "50" });
  if (cursor) query.set("cursor", cursor);
  if (status) query.set("status", status);
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
  await activeCmaBatches(
    ownerId,
    connection,
    routes,
    signal,
    async (agents) => {
      for (const agent of agents)
        if (ACTIVE_STATUSES.includes(agent.status))
          sessions.set(agent.id, {
            connection,
            routes,
            agentId: agent.id,
            title: (agent.owner_title || agent.title || "CMA session").slice(
              0,
              300,
            ),
          });
      return false;
    },
  );
  return [...sessions.values()];
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
  session: Pick<CmaSessionReference, "title">,
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
  const revision = getAssistantIdentityRevision();
  let state = initialCmaSession(session);
  let cursor: string | undefined;
  let incarnation: string | undefined;
  let sequence = 0;
  let failures = 0;
  const publish = (change: Partial<CmaSessionState>) => {
    if (
      signal.aborted ||
      getAssistantIdentityUserId() !== ownerId ||
      getAssistantIdentityRevision() !== revision
    )
      return;
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
      guard(ownerId, revision);
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
        guard(ownerId, revision);
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
          guard(ownerId, revision);
          if (signal.aborted) return;
          state = next;
          update(state);
          sequence = event.sequence;
          cursor = event.cursor;
        }
      }
    } catch (error) {
      if (
        signal.aborted ||
        getAssistantIdentityUserId() !== ownerId ||
        getAssistantIdentityRevision() !== revision
      )
        return;
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

/** Only server receipts establish origin; inventory never supplies attribution. */
export async function listCmaSessionOrigins(
  ownerId: string,
  signal: AbortSignal,
  conversation?: string,
  pins: readonly string[] = [],
  agents?: readonly string[],
): Promise<CmaSessionOrigin[]> {
  if (!conversation && !pins.length) return [];
  const scope = new URLSearchParams();
  if (conversation) scope.set("conversation", conversation);
  if (pins.length) scope.set("pins", pins.slice(0, 50).join(","));
  if (agents) scope.set("agents", agents.slice(0, 50).join(","));
  const origins = new Map<string, CmaSessionOrigin>();
  const cursors = new Set<string>();
  let after: string | null = null;
  // Production callers supply at most fifty exact IDs, so historical receipts
  // never determine the page count of active discovery.
  for (let page = 0; page < 100; page++) {
    if (after) scope.set("after", after);
    const result = cmaOriginPageSchema.parse(
      await json(ownerId, `/assistant/nyxagent/cma-sessions?${scope}`, signal),
    );
    for (const origin of result.sessions)
      origins.set(`${origin.service_id}:${origin.agent_id}`, origin);
    if (!result.next_cursor) return [...origins.values()];
    if (cursors.has(result.next_cursor))
      throw new Error("CMA repeated an origin cursor.");
    cursors.add(result.next_cursor);
    after = result.next_cursor;
  }
  throw new Error("CMA origin discovery exceeded its page limit.");
}

export function isCmaAccessError(error: unknown): boolean {
  return error instanceof ApiError && [401, 403, 404].includes(error.status);
}

// Ephemeral scheduling only, scoped by the caller to the current account/source.
// Resume the next unvisited eligible connection after a collection deadline.
export interface CmaDiscoveryProgress {
  ownerId?: string;
  conversationId?: string;
  after?: string;
  nextConnection?: string;
}

type RouteResolver = (id: string, signal: AbortSignal) => Promise<CmaRoutes>;

/** Receipt reads are exact and precede any pinned detail/stream request. */
export async function listPinnedCmaSessions(
  ownerId: string,
  signal: AbortSignal,
  pins: readonly string[],
  getRoutes?: RouteResolver,
  previous: readonly CmaSessionReference[] = [],
): Promise<CmaSessionReference[]> {
  if (!pins.length) return [];
  const revision = getAssistantIdentityRevision();
  const started = Date.now();
  const bounded = AbortSignal.any([signal, AbortSignal.timeout(25_000)]);
  const origins = await listCmaSessionOrigins(
    ownerId,
    bounded,
    undefined,
    pins,
    pins,
  );
  const connections = [...new Set(origins.map((origin) => origin.service_id))];
  const sessions = new Map<string, CmaSessionReference>();
  for (const origin of origins) {
    const prior = previous.find(
      (session) =>
        session.connection.id === origin.service_id &&
        session.agentId === origin.agent_id,
    );
    if (prior)
      sessions.set(`${origin.service_id}:${origin.agent_id}`, {
        ...prior,
        origin,
      });
  }
  // Allocate the bounded route budget across all pins, so a late healthy pin
  // cannot wait behind successive ten-second peers.
  const routeBudget = Math.min(
    10_000,
    Math.max(1, 24_000 - (Date.now() - started)) /
      Math.max(1, Math.ceil(connections.length / 3)),
  );
  await visitConnections(connections, bounded, async (id) => {
    const authorized = origins.filter(
      (origin) =>
        origin.service_id === id && pins.includes(`${id}:${origin.agent_id}`),
    );
    try {
      const peer = AbortSignal.any([
        bounded,
        AbortSignal.timeout(Math.floor(routeBudget)),
      ]);
      const routes = await (getRoutes?.(id, peer) ??
        resolveCmaRoutes(ownerId, id, peer));
      for (const origin of authorized)
        sessions.set(`${id}:${origin.agent_id}`, {
          origin,
          connection: {
            id,
            name: origin.connection_name ?? "CMA",
            slug: origin.connection_slug ?? "cma",
            connected: true,
            requires_connection: true,
          },
          routes,
          agentId: origin.agent_id,
          title: "CMA session",
        });
    } catch (error) {
      guard(ownerId, revision);
      signal.throwIfAborted();
      if (!isCmaRetryableError(error))
        for (const origin of authorized)
          sessions.delete(`${id}:${origin.agent_id}`);
    }
  });
  guard(ownerId, revision);
  signal.throwIfAborted();
  return [...sessions.values()];
}

/** Each page uses CMA's single-enum status filter. Every candidate batch is
 * joined to authoritative receipts before counting toward the card limit. */
export async function discoverCmaSessions(
  ownerId: string,
  signal: AbortSignal,
  conversation: string,
  getRoutes?: RouteResolver,
  previous: readonly CmaSessionReference[] = [],
  progress: CmaDiscoveryProgress = {},
): Promise<CmaSessionReference[]> {
  const revision = getAssistantIdentityRevision();
  const deadline = AbortSignal.timeout(25_000);
  const bounded = AbortSignal.any([signal, deadline]);
  const sessions = new Map<string, CmaSessionReference>();
  // Exact prior receipt authorization is independent of this poll's inventory
  // position. A skipped/slow peer cannot revive a withdrawn receipt.
  const prior = previous
    .filter((session) => session.origin?.conversation_id === conversation)
    .slice(0, 50);
  for (const session of prior)
    sessions.set(`${session.connection.id}:${session.agentId}`, session);
  const scope = new URLSearchParams({ discover: "true", conversation });
  if (progress.after) scope.set("after", progress.after);
  const connectionCursors = new Set<string>();
  try {
    if (prior.length) {
      const origins = await listCmaSessionOrigins(
        ownerId,
        bounded,
        conversation,
        [],
        [...sessions.keys()],
      );
      for (const [key, session] of sessions) {
        const origin = origins.find(
          (origin) =>
            `${origin.service_id}:${origin.agent_id}` === key &&
            origin.conversation_id === conversation,
        );
        if (origin) sessions.set(key, { ...session, origin });
        else sessions.delete(key);
      }
    }
    for (let connectionPage = 0; connectionPage < 100; connectionPage++) {
      const page = cmaOriginPageSchema.parse(
        await json(
          ownerId,
          `/assistant/nyxagent/cma-sessions?${scope}`,
          bounded,
        ),
      );
      const start = Math.max(
        0,
        page.connections.findIndex(
          (connection) => connection.id === progress.nextConnection,
        ),
      );
      const remaining = page.connections.slice(start);
      const next = await visitConnections(
        remaining,
        bounded,
        async (connection) => {
          const found = new Map<string, CmaSessionReference>();
          const peer = AbortSignal.any([bounded, AbortSignal.timeout(10_000)]);
          try {
            const routes = await (getRoutes?.(connection.id, peer) ??
              resolveCmaRoutes(ownerId, connection.id, peer));
            await activeCmaBatches(
              ownerId,
              connection,
              routes,
              peer,
              async (agents) => {
                const active = agents.filter((agent) =>
                  ACTIVE_STATUSES.includes(agent.status),
                );
                if (!active.length) return false;
                const origins = await listCmaSessionOrigins(
                  ownerId,
                  peer,
                  conversation,
                  [],
                  active.map((agent) => `${connection.id}:${agent.id}`),
                );
                for (const origin of origins) {
                  if (origin.conversation_id !== conversation) continue;
                  const agent = active.find(
                    (agent) => agent.id === origin.agent_id,
                  );
                  if (!agent) continue;
                  found.set(`${connection.id}:${agent.id}`, {
                    origin,
                    connection,
                    routes,
                    agentId: agent.id,
                    title: (
                      agent.owner_title ||
                      agent.title ||
                      "CMA session"
                    ).slice(0, 300),
                  });
                }
                return found.size >= 50;
              },
            );
          } catch (error) {
            guard(ownerId, revision);
            signal.throwIfAborted();
            if (error instanceof CmaAccountAuthorizationError) throw error;
            // Retryable failures preserve completed, receipt-verified work as
            // well as this poll's authorized prior rows.
            // Confirmed denials remove their own rows even if the collection times out.
            if (isCmaRetryableError(error)) {
              for (const [key, session] of found) sessions.set(key, session);
              return;
            }
            found.clear();
          }
          for (const [key, session] of sessions)
            if (session.connection.id === connection.id) sessions.delete(key);
          for (const [key, session] of found) sessions.set(key, session);
        },
      );
      guard(ownerId, revision);
      signal.throwIfAborted();
      if (next < remaining.length) {
        progress.nextConnection = remaining[next]!.id;
        break;
      }
      progress.nextConnection = undefined;
      progress.after = page.next_cursor ?? undefined;
      if (!page.next_cursor) break;
      if (connectionCursors.has(page.next_cursor))
        throw new Error("CMA repeated a connection cursor.");
      connectionCursors.add(page.next_cursor);
      scope.set("after", page.next_cursor);
      if (sessions.size >= 50) break;
    }
  } catch (error) {
    guard(ownerId, revision);
    signal.throwIfAborted();
    // The local collection deadline preserves completed peers and known denials;
    // caller cancellation/account changes still reject the entire observation.
    if (!deadline.aborted || error instanceof CmaAccountAuthorizationError)
      throw error;
  }
  guard(ownerId, revision);
  signal.throwIfAborted();
  return [...sessions.values()].slice(0, 50);
}

/** Work-conserving bounded pool. Return the first unstarted peer on a deadline. */
async function visitConnections<T>(
  connections: readonly T[],
  signal: AbortSignal,
  visit: (connection: T) => Promise<void>,
): Promise<number> {
  let next = 0;
  const outcomes = await Promise.allSettled(
    Array.from({ length: Math.min(3, connections.length) }, async () => {
      while (next < connections.length && !signal.aborted) {
        const connection = connections[next++]!;
        await visit(connection);
      }
    }),
  );
  for (const outcome of outcomes)
    if (outcome.status === "rejected") throw outcome.reason;
  return next;
}

const ACTIVE_STATUSES = ["starting", "working", "waiting_approval", "paused"];
async function activeCmaBatches(
  ownerId: string,
  connection: CmaConnection,
  routes: CmaRoutes,
  signal: AbortSignal,
  visit: (agents: z.infer<typeof cmaAgentSchema>[]) => Promise<boolean>,
) {
  for (const status of ACTIVE_STATUSES) {
    let cursor: string | undefined;
    const cursors = new Set<string>();
    for (let page = 0; page < 100; page++) {
      const result = await listCmaAgents(
        ownerId,
        connection.id,
        routes,
        signal,
        cursor,
        status,
      );
      if (await visit(result.items)) return;
      if (!result.next) break;
      if (cursors.has(result.next))
        throw new Error("CMA repeated a session list cursor.");
      cursors.add(result.next);
      cursor = result.next;
      if (page === 99)
        throw new Error(
          "CMA active session discovery exceeded its page limit.",
        );
    }
  }
}

export function isCmaRetryableError(error: unknown): boolean {
  return (
    error instanceof TypeError ||
    (error instanceof DOMException &&
      ["TimeoutError", "AbortError"].includes(error.name)) ||
    (error instanceof ApiError && (error.status === 429 || error.status >= 500))
  );
}
