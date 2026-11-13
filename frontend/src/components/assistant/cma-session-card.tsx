import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { getFormattingLocale } from "@/lib/formatting-locale";
import {
  useLayoutEffect,
  useId,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from "react";
import {
  Check,
  ChevronDown,
  Circle,
  Loader2,
  Maximize2,
  Minimize2,
  RefreshCw,
  Pin,
  ExternalLink,
  MessageSquare,
  X,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipTrigger,
  TooltipContent,
} from "@/components/ui/tooltip";
import { CmaGlyph } from "@/components/service-icons/cma";
import {
  CmaExecutionControl,
  CmaOperationCard,
  CmaSessionRequests,
} from "./cma-operation-card";
import type { CmaOperation } from "@/schemas/cma-operation";
import { Link, useRouterState } from "@tanstack/react-router";
import { safeCmaUrl } from "@/lib/assistant/cma-url";
import {
  useObservedCmaSessions,
  type ObservedCmaSession,
} from "./cma-session-context";
import { useAuthStore } from "@/stores/auth-store";
import { useCmaSessionStore } from "@/stores/cma-session-store";
import type {
  CmaSessionReference,
  CmaSessionState,
  CmaSessionStatus,
} from "@/schemas/cma-session";
import { cn } from "@/lib/utils";

const STATUS_LABELS: Record<CmaSessionStatus, string> = {
  starting: "Starting",
  working: "Working",
  ready: "Ready",
  completed: "Completed",
  waiting: "Waiting for you",
  paused: "Paused",
  failed: "Failed",
  stopped: "Stopped",
  deleted: "Deleted",
  unknown: "Checking status",
};

export function CmaSessionCards() {
  useUiTranslation();
  return <CmaSessionObserver>{(cards) => cards}</CmaSessionObserver>;
}

export function CmaSessionObserver({
  children,
  operations = [],
}: {
  readonly children: (cards: ReactNode) => ReactNode;
  readonly operations?: readonly CmaOperation[];
}) {
  useUiTranslation();
  const ownerId = useAuthStore((state) => state.user?.id);
  const sessions = useObservedCmaSessions();
  const matchingSession = (operation: CmaOperation, session: CmaSessionReference) =>
    operation.service_id === session.connection.id &&
    operation.agent_id === session.agentId &&
    (!operation.reason || operation.reason === "completed");
  const [receiptLimit, setReceiptLimit] = useState(3);
  const receipts = operations.filter(
    (operation) =>
      !sessions.some(({ session }) => matchingSession(operation, session)),
  );
  return children(
    ownerId && (sessions.length || receipts.length) ? (
      <aside
        aria-label={translateUi("CMA sessions")}
        className="flex min-h-0 shrink-0 flex-col gap-3"
      >
        {sessions.map((observation) => (
          <CmaSessionCard
            key={`${observation.session.connection.id}:${observation.session.agentId}`}
            ownerId={ownerId}
            observation={observation}
            operations={operations.filter((operation) => matchingSession(operation, observation.session))}
          />
        ))}
        {receipts.slice(0, receiptLimit).map((operation) => (
          <CmaOperationCard key={operation.id} operation={operation} />
        ))}
        {receipts.length > receiptLimit ? (
          <Button variant="ghost" size="sm" onClick={() => setReceiptLimit((value) => value + 3)}>{translateUi("Show older CMA submissions (")}{receipts.length - receiptLimit})
          </Button>
        ) : null}
      </aside>
    ) : null,
  );
}

/** Every route outside the assistant shell gets the same pinned cards. */
export function FloatingCmaSessions() {
  useUiTranslation();
  const assistant = useRouterState({
    select: (state) =>
      !state.matches.some(
        (match) => match.globalNotFound || match.status === "error",
      ) &&
      state.matches.some(
        (match) =>
          [
            "/assistant",
            "/assistant/plugins",
            "/assistant/approvals",
            "/assistant/automations",
            "/assistant/machines",
            "/assistant/machines/new",
            "/assistant/machines/pair",
            "/assistant/sandboxes",
            "/assistant/sandboxes/$sandboxId",
            "/assistant/cma-agents/$agentId",
          ].includes(match.routeId) &&
          match.status === "success" &&
          match.pathname.replace(/\/$/, "") ===
            state.location.pathname.replace(/\/$/, ""),
      ),
  });
  if (assistant) return null;
  return (
    <CmaSessionObserver>
      {(cards) =>
        cards ? (
          <div className="fixed bottom-[max(1rem,var(--sab))] right-[max(1rem,var(--sar))] z-40 max-h-[min(60dvh,560px)] w-[min(360px,calc(100vw-2rem))] overflow-y-auto rounded-xl shadow-lg">
            {cards}
          </div>
        ) : null
      }
    </CmaSessionObserver>
  );
}

/** Current-session activity only; pinned work from other sessions is excluded. */
export function CmaSessionIndicator({
  conversationId,
}: {
  readonly conversationId: string;
}) {
  useUiTranslation();
  const sessions = useObservedCmaSessions().filter(
    ({ session }) => session.origin?.conversation_id === conversationId,
  );
  if (!sessions.length) return null;
  const working = sessions.some(
    ({ state }) => state.status === "working" || state.status === "starting",
  );
  return (
    <span
      role="status"
      aria-label={
        working
          ? translateUi("CMA is working in this session")
          : translateUi("CMA activity in this session")
      }
      title={translateUi("{{count}} CMA sessions", { count: sessions.length })}
      className="inline-flex min-h-7 items-center gap-1.5 rounded-full border border-border px-2 py-0.5 type-group-label text-muted-foreground"
    >
      <CmaGlyph aria-hidden="true" className="h-3.5 w-3.5" />{translateUi(" CMA")}<span
        aria-hidden="true"
        className={cn(
          "h-1.5 w-1.5 rounded-full",
          working ? "bg-success motion-safe:animate-pulse" : "bg-warning",
        )}
      />
    </span>
  );
}

function CmaSessionCard({
  ownerId,
  observation,
  operations,
}: {
  readonly operations: readonly CmaOperation[];
  readonly ownerId: string;
  readonly observation: ObservedCmaSession;
}) {
  useUiTranslation();
  const { session, state, reconnect } = observation;
  const key = `${session.connection.id}:${session.agentId}`;
  const collapsed = useCmaSessionStore(
    (store) => store.ownerId === ownerId && store.minimised[key] === true,
  );
  const setMinimised = useCmaSessionStore((store) => store.setMinimised);
  const pinned = useCmaSessionStore(
    (store) => store.ownerId === ownerId && store.pinned[key] === true,
  );
  const setPinned = useCmaSessionStore((store) => store.setPinned);
  const cmaUrl = safeCmaUrl(
    session.callbackUrl === undefined
      ? session.origin?.cma_url
      : session.callbackUrl,
  );
  const detailsId = `cma-session-${session.connection.id}-${session.agentId}`;
  const scrollBody = useRef<HTMLDivElement>(null);
  const activeActionId =
    state.status === "working" || state.status === "starting"
      ? [...state.actions]
          .reverse()
          .find((action) => action.status === "running")?.id
      : undefined;
  return (
    <section
      aria-label={translateUi("CMA session: {{value1}}", { value1: state.title })}
      data-private="true"
      data-ph-no-capture
      className="flex max-h-[min(440px,40dvh)] min-h-0 w-full max-w-[500px] shrink-0 flex-col self-start gap-2 overflow-hidden rounded-xl border border-border/50 bg-card p-3 text-14 lg:max-h-[480px] lg:rounded-2xl lg:border-0 lg:shadow-sm"
    >
      <div className="flex shrink-0 flex-wrap items-center gap-x-2 gap-y-1">
        <CmaGlyph
          aria-hidden="true"
          className="h-4 w-4 shrink-0 text-muted-foreground"
        />
        <h2
          className="min-w-0 flex-1 basis-32 line-clamp-2 break-words type-card-title"
          title={state.title}
        >
          {state.title}
        </h2>
        <div
          className="ml-auto flex max-w-full shrink-0 flex-wrap items-center justify-end gap-1"
          role="group"
          aria-label={translateUi("CMA session controls")}
        >
          <CmaExecutionControl operations={operations} />
          {session.origin ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  asChild
                  variant="ghost"
                  size="icon"
                  className="text-foreground hover:bg-overlay-strong focus-visible:ring-2 focus-visible:ring-foreground focus-visible:ring-offset-2 focus-visible:ring-offset-card"
                >
                  <Link
                    to="/assistant"
                    search={{
                      c: session.origin.conversation_id,
                      agent: session.origin.specialist_id,
                    }}
                    aria-label={translateUi("Open source chat")}
                  >
                    <MessageSquare aria-hidden="true" />
                  </Link>
                </Button>
              </TooltipTrigger>
              <TooltipContent>{translateUi("Open source chat")}</TooltipContent>
            </Tooltip>
          ) : null}
          {cmaUrl ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  asChild
                  variant="ghost"
                  size="icon"
                  className="text-foreground hover:bg-overlay-strong focus-visible:ring-2 focus-visible:ring-foreground focus-visible:ring-offset-2 focus-visible:ring-offset-card"
                >
                  <a
                    href={cmaUrl}
                    target="_blank"
                    rel="noopener noreferrer"
                    aria-label={translateUi("View in CMA")}
                  >
                    <ExternalLink aria-hidden="true" />
                  </a>
                </Button>
              </TooltipTrigger>
              <TooltipContent>{translateUi("View in CMA")}</TooltipContent>
            </Tooltip>
          ) : null}
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size={pinned ? "sm" : "icon"}
                aria-label={
                  pinned ? translateUi("Unpin CMA session (Pinned)") : translateUi("Pin CMA session")
                }
                aria-pressed={pinned}
                onClick={() => setPinned(ownerId, key, !pinned)}
                className={cn(
                  "h-8 shrink-0 transition-shadow focus-visible:ring-2 focus-visible:ring-foreground focus-visible:ring-offset-2 focus-visible:ring-offset-card",
                  pinned
                    ? "bg-foreground px-2 text-background hover:bg-foreground hover:text-background"
                    : "text-foreground hover:bg-overlay-strong",
                )}
              >
                <Pin
                  aria-hidden="true"
                  className={pinned ? "fill-current" : undefined}
                />
                {pinned ? (
                  <span className="whitespace-nowrap">{translateUi("Pinned")}</span>
                ) : null}
              </Button>
            </TooltipTrigger>
            <TooltipContent>
              {pinned
                ? translateUi("Unpin: show only in the source chat")
                : translateUi("Pin to monitor on every page")}
            </TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                aria-label={
                  collapsed ? translateUi("Maximise CMA session") : translateUi("Minimise CMA session")
                }
                aria-expanded={!collapsed}
                aria-controls={detailsId}
                onClick={() => setMinimised(ownerId, key, !collapsed)}
                className="shrink-0 text-foreground hover:bg-overlay-strong focus-visible:ring-2 focus-visible:ring-foreground focus-visible:ring-offset-2 focus-visible:ring-offset-card"
              >
                {collapsed ? <Maximize2 /> : <Minimize2 />}
              </Button>
            </TooltipTrigger>
            <TooltipContent>
              {collapsed ? translateUi("Maximise CMA session") : translateUi("Minimise CMA session")}
            </TooltipContent>
          </Tooltip>
        </div>
      </div>
      <div ref={scrollBody} className="min-h-0 space-y-2 overflow-y-auto">
        <SessionStatus state={state} animate={collapsed || !activeActionId} />
        {collapsed && state.actions.length ? (
          <p className="truncate text-13 text-muted-foreground">
            {state.actions.at(-1)?.title}
          </p>
        ) : null}
        {state.error ? (
          <div className="space-y-2 rounded-lg border border-warning/25 bg-warning/5 p-2 text-13">
            <p role="alert">{state.error}</p>
            {state.connection === "unavailable" ? (
              <Button
                variant="outline"
                size="sm"
                onClick={reconnect}
                className="focus-visible:ring-2 focus-visible:ring-ring"
              >
                <RefreshCw />{translateUi(" Reconnect")}</Button>
            ) : null}
          </div>
        ) : null}
        <div id={detailsId} hidden={collapsed} className="space-y-2">
          <SessionActivity
            state={state}
            activeActionId={collapsed ? undefined : activeActionId}
          />
          <CmaSessionRequests operations={operations} scrollBody={scrollBody} visible={!collapsed} />
          <SessionDetails
            session={session}
            state={state}
            scrollBody={scrollBody}
            visible={!collapsed}
          />
        </div>
      </div>
    </section>
  );
}

function SessionDetails({
  session,
  state,
  scrollBody,
  visible,
}: {
  readonly session: CmaSessionReference;
  readonly state: CmaSessionState;
  readonly scrollBody: RefObject<HTMLDivElement | null>;
  readonly visible: boolean;
}) {
  useUiTranslation();
  const [open, setOpen] = useState(false);
  const content = useRef<HTMLDivElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  const contentId = `cma-session-metadata-${session.connection.id}-${session.agentId}`;
  useLayoutEffect(() => {
    if (
      !open ||
      !visible ||
      !content.current ||
      !button.current ||
      !scrollBody.current
    )
      return;
    const body = scrollBody.current;
    const viewport = body.getBoundingClientRect();
    const top = button.current.getBoundingClientRect().top;
    const bottom = content.current.getBoundingClientRect().bottom;
    // Keep the disclosure and its content together when the activity list fills
    // the card. Scroll only this card, leaving the conversation in place.
    body.scrollBy({
      top: Math.min(top - viewport.top, Math.max(0, bottom - viewport.bottom)),
      behavior: "instant",
    });
  }, [open, visible, scrollBody]);

  return (
    <div className="pt-1 text-13 text-text-tertiary">
      <Button
        ref={button}
        type="button"
        variant="ghost"
        size="sm"
        aria-expanded={open}
        aria-controls={contentId}
        onClick={() => setOpen((value) => !value)}
        className="w-full justify-between px-0 focus-visible:ring-2 focus-visible:ring-ring"
      >{translateUi("Session details")}<ChevronDown
          aria-hidden="true"
          className={cn(
            "transition-transform motion-reduce:transition-none",
            open && "rotate-180",
          )}
        />
      </Button>
      <div ref={content} id={contentId} hidden={!open}>
        <dl className="space-y-2 px-1 pb-1 pt-2">
          <div>
            <dt>{translateUi("Connection")}</dt>
            <dd className="mt-0.5 break-words text-muted-foreground">
              {session.connection.name}
            </dd>
          </div>
          {state.updatedAt ? (
            <div>
              <dt>{translateUi("Last activity")}</dt>
              <dd className="mt-0.5 text-muted-foreground">
                <time dateTime={state.updatedAt} aria-label={translateUi("Last CMA activity")}>
                  {new Date(state.updatedAt).toLocaleString(getFormattingLocale())}
                </time>
              </dd>
            </div>
          ) : null}
          <div>
            <dt>{translateUi("Session ID")}</dt>
            <dd className="mt-0.5 break-all font-mono text-muted-foreground">
              {session.agentId}
            </dd>
          </div>
        </dl>
      </div>
    </div>
  );
}

function SessionStatus({
  state,
  animate,
}: {
  readonly state: CmaSessionState;
  readonly animate: boolean;
}) {
  useUiTranslation();
  const live = state.connection === "live";
  return (
    <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
      <span className="text-13 text-muted-foreground">{translateUi("Agent:")}</span>
      <span
        role="status"
        aria-label={translateUi("CMA agent status: {{value1}}", { value1: STATUS_LABELS[state.status] })}
        className="flex items-center gap-1.5 text-13 text-foreground"
      >
        {animate && (state.status === "working" || state.status === "starting") ? (
          <Loader2
            aria-hidden="true"
            className="h-3.5 w-3.5 animate-spin motion-reduce:animate-none text-primary-text"
          />
        ) : (
          <Circle
            className={cn(
              "h-2 w-2 fill-current",
              state.status === "failed"
                ? "text-destructive"
                : "text-text-tertiary",
            )}
          />
        )}
        {STATUS_LABELS[state.status]}
      </span>
      <span aria-hidden="true" className="text-text-tertiary">
        ·
      </span>
      <span
        className="flex items-center gap-1.5 text-13 text-muted-foreground"
        aria-label={translateUi("Status stream {{value1}}", { value1: state.connection })}
      >
        <span
          className={cn(
            "h-1.5 w-1.5 rounded-full",
            live
              ? "bg-success"
              : state.connection === "unavailable"
                ? "bg-destructive"
                : "bg-warning",
          )}
        />
        {live
          ? translateUi("Live")
          : state.connection === "reconnecting"
            ? translateUi("Reconnecting")
            : state.connection === "unavailable"
              ? translateUi("Disconnected")
              : translateUi("Connecting")}
      </span>
    </div>
  );
}

function SessionActivity({
  state,
  activeActionId,
}: {
  readonly state: CmaSessionState;
  readonly activeActionId?: string;
}) {
  useUiTranslation();
  const [expanded, setExpanded] = useState(false);
  const actions = [...state.actions].reverse();
  const historyId = useId();
  return (
    <div className="space-y-1.5">
      {state.notice ? (
        <p
          role="note"
          className="text-13 text-muted-foreground"
        >
          {state.notice}
        </p>
      ) : null}
      <section aria-label={translateUi("CMA activity")}>
        <div className="mb-0.5 flex flex-wrap items-center justify-between gap-x-2 gap-y-1">
          <h4 className="type-group-label text-text-tertiary">{translateUi("Recent actions")}</h4>
          {actions.length > 3 ? (
            <Button
              variant="ghost"
              size="sm"
              className="max-w-full shrink-0 whitespace-normal break-words px-1 text-left"
              aria-expanded={expanded}
              aria-controls={historyId}
              onClick={() => setExpanded((value) => !value)}
            >
              {expanded
                ? translateUi("Show recent actions")
                : translateUi("Show all {{value1}} actions", { value1: actions.length })}
            </Button>
          ) : null}
        </div>
        {state.actions.length ? (
          <ol id={historyId}>
            {(expanded ? actions : actions.slice(0, 3)).map((action) => (
              <li
                key={action.id}
                className="flex items-start gap-1.5 py-1 text-13"
              >
                <span className="mt-0.5 shrink-0" aria-label={action.status}>
                  {action.id === activeActionId ? (
                    <Loader2 className="h-3 w-3 animate-spin motion-reduce:animate-none text-primary-text" />
                  ) : action.status === "completed" ? (
                    <Check className="h-3 w-3 text-success" />
                  ) : action.status === "failed" ? (
                    <X className="h-3 w-3 text-destructive" />
                  ) : (
                    <Circle
                      className={cn(
                        "h-3 w-3",
                        action.status === "running"
                          ? "text-primary-text"
                          : "text-text-tertiary",
                      )}
                    />
                  )}
                </span>
                <span className="min-w-0 break-words text-muted-foreground">
                  {action.title}
                </span>
              </li>
            ))}
          </ol>
        ) : (
          <p className="text-13 text-text-tertiary">
            {state.status === "working"
              ? translateUi("CMA is working. Actions will appear as it reports them.")
              : translateUi("New session activity will appear here.")}
          </p>
        )}
      </section>
    </div>
  );
}
