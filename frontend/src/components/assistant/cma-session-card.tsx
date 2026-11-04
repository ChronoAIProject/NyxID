import { useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from "react";
import {
  Check,
  ChevronDown,
  Circle,
  Loader2,
  Maximize2,
  Minimize2,
  RefreshCw,
  X,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { CmaGlyph } from "@/components/service-icons/cma";
import { useAuthStore } from "@/stores/auth-store";
import { useCmaSessionStore } from "@/stores/cma-session-store";
import {
  useActiveCmaSessions,
  useCmaSessionStream,
} from "@/hooks/use-cma-session";
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
  return <CmaSessionObserver>{(cards) => cards}</CmaSessionObserver>;
}

export function CmaSessionObserver({ children }: {
  readonly children: (cards: ReactNode) => ReactNode;
}) {
  const ownerId = useAuthStore((state) => state.user?.id);
  return ownerId ? (
    <OwnedCmaSessionCards key={ownerId} ownerId={ownerId}>{children}</OwnedCmaSessionCards>
  ) : children(null);
}

function OwnedCmaSessionCards({ ownerId, children }: {
  readonly ownerId: string;
  readonly children: (cards: ReactNode) => ReactNode;
}) {
  const sessions = useActiveCmaSessions(ownerId);
  if (!sessions.length) return children(null);
  return children(
    <aside
      aria-label="CMA sessions"
      className="flex min-h-0 shrink-0 flex-col gap-3"
    >
      {sessions.map((session) => (
        <CmaSessionCard
          key={`${session.connection.id}:${session.agentId}`}
          ownerId={ownerId}
          session={session}
        />
      ))}
    </aside>
  );
}

function CmaSessionCard({
  ownerId,
  session,
}: {
  readonly ownerId: string;
  readonly session: CmaSessionReference;
}) {
  const key = `${session.connection.id}:${session.agentId}`;
  const collapsed = useCmaSessionStore(
    (store) => store.ownerId === ownerId && store.minimised[key] === true,
  );
  const setMinimised = useCmaSessionStore((store) => store.setMinimised);
  const { state, reconnect } = useCmaSessionStream(ownerId, session);
  const detailsId = `cma-session-${session.connection.id}-${session.agentId}`;
  const scrollBody = useRef<HTMLDivElement>(null);
  return (
    <section
      aria-label={`CMA session: ${state.title}`}
      data-private="true"
      data-ph-no-capture
      className="flex max-h-[min(440px,40dvh)] min-h-0 shrink-0 flex-col gap-3 overflow-hidden rounded-xl border border-border/50 bg-card p-3 text-14 lg:max-h-[480px] lg:rounded-l-2xl lg:rounded-r-none lg:border-0 lg:shadow-sm"
    >
      <div className="flex shrink-0 items-center gap-2">
        <CmaGlyph className="h-4 w-4 text-muted-foreground" />
        <h2 className="flex-1 font-medium">Your CMA session</h2>
        <span className="text-12 text-text-tertiary">Across chats</span>
        <Button
          variant="ghost"
          size="sm"
          aria-label={
            collapsed ? "Maximise CMA session" : "Minimise CMA session"
          }
          aria-expanded={!collapsed}
          aria-controls={detailsId}
          onClick={() => setMinimised(ownerId, key, !collapsed)}
          className="h-7 w-7 shrink-0 px-0"
        >
          {collapsed ? <Maximize2 /> : <Minimize2 />}
        </Button>
      </div>
      <div className="shrink-0 space-y-2">
        <h3
          className="line-clamp-2 break-words font-medium leading-relaxed"
          title={state.title}
        >
          {state.title}
        </h3>
        <SessionStatus state={state} />
        {collapsed && state.actions.length ? (
          <p className="truncate text-13 text-muted-foreground">
            {state.actions.at(-1)?.title}
          </p>
        ) : null}
      </div>
      {state.error ? (
        <div className="shrink-0 space-y-2 rounded-lg border border-warning/25 bg-warning/5 p-2 text-13">
          <p role="alert">{state.error}</p>
          {state.connection === "unavailable" ? (
            <Button variant="outline" size="sm" onClick={reconnect}>
              <RefreshCw /> Reconnect
            </Button>
          ) : null}
        </div>
      ) : null}
      <div
        id={detailsId}
        ref={scrollBody}
        className={cn(
          "min-h-0 space-y-3 overflow-y-auto",
          collapsed && "hidden",
        )}
      >
        <SessionActivity state={state} />
        <SessionDetails
          session={session}
          scrollBody={scrollBody}
          visible={!collapsed}
        />
      </div>
    </section>
  );
}

function SessionDetails({
  session,
  scrollBody,
  visible,
}: {
  readonly session: CmaSessionReference;
  readonly scrollBody: RefObject<HTMLDivElement | null>;
  readonly visible: boolean;
}) {
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
    <div className="border-t border-border/50 pt-1 text-12 text-text-tertiary">
      <Button
        ref={button}
        type="button"
        variant="ghost"
        size="sm"
        aria-expanded={open}
        aria-controls={contentId}
        onClick={() => setOpen((value) => !value)}
        className="w-full justify-between px-1 text-12 focus-visible:ring-2 focus-visible:ring-ring"
      >
        Session details
        <ChevronDown
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
            <dt>Connection</dt>
            <dd className="mt-0.5 break-words text-muted-foreground">
              {session.connection.name}
            </dd>
          </div>
          <div>
            <dt>Session ID</dt>
            <dd className="mt-0.5 break-all font-mono text-muted-foreground">
              {session.agentId}
            </dd>
          </div>
        </dl>
      </div>
    </div>
  );
}

function SessionStatus({ state }: { readonly state: CmaSessionState }) {
  const live = state.connection === "live";
  return (
    <div className="flex items-center justify-between gap-2">
      <span
        role="status"
        className="flex items-center gap-2 text-13 text-muted-foreground"
      >
        {state.status === "working" || state.status === "starting" ? (
          <Loader2 className="h-3.5 w-3.5 animate-spin motion-reduce:animate-none text-primary" />
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
      <span
        className="flex items-center gap-1.5 text-12 text-text-tertiary"
        aria-label={`Status stream ${state.connection}`}
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
          ? "Live"
          : state.connection === "reconnecting"
            ? "Reconnecting"
            : state.connection === "unavailable"
              ? "Disconnected"
              : "Connecting"}
      </span>
    </div>
  );
}

function SessionActivity({ state }: { readonly state: CmaSessionState }) {
  return (
    <div className="space-y-3">
      {state.notice ? (
        <p
          role="note"
          className="text-13 leading-relaxed text-muted-foreground"
        >
          {state.notice}
        </p>
      ) : null}
      <section aria-label="CMA activity">
        <div className="mb-1 flex items-center justify-between gap-2">
          <h4 className="text-12 font-medium text-text-tertiary">
            Recent actions
          </h4>
          {state.updatedAt ? (
            <time
              dateTime={state.updatedAt}
              aria-label="Last CMA activity"
              className="text-12 tabular-nums text-text-tertiary"
            >
              {new Date(state.updatedAt).toLocaleTimeString([], {
                hour: "2-digit",
                minute: "2-digit",
                second: "2-digit",
              })}
            </time>
          ) : null}
        </div>
        {state.actions.length ? (
          <ol className="space-y-0">
            {[...state.actions].reverse().map((action) => (
              <li
                key={action.id}
                className="flex items-start gap-2 rounded-lg bg-overlay/35 px-2 py-2 text-13 [&+li]:mt-1"
              >
                <span className="mt-0.5 shrink-0" aria-label={action.status}>
                  {action.status === "running" ? (
                    <Loader2 className="h-3 w-3 animate-spin motion-reduce:animate-none text-primary" />
                  ) : action.status === "completed" ? (
                    <Check className="h-3 w-3 text-success" />
                  ) : action.status === "failed" ? (
                    <X className="h-3 w-3 text-destructive" />
                  ) : (
                    <Circle className="h-3 w-3 text-text-tertiary" />
                  )}
                </span>
                <span className="min-w-0 break-words leading-relaxed text-muted-foreground">
                  {action.title}
                </span>
              </li>
            ))}
          </ol>
        ) : (
          <p className="text-13 leading-relaxed text-text-tertiary">
            {state.status === "working"
              ? "CMA is working. Actions will appear as it reports them."
              : "New session activity will appear here."}
          </p>
        )}
      </section>
    </div>
  );
}
