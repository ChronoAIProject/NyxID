import { Link } from "@tanstack/react-router";
import { useState } from "react";
import { cn, formatRelativeTime } from "@/lib/utils";
import {
  useServiceHistory,
  useArchivedServiceHistory,
} from "@/hooks/use-service-history";
import type { ServiceAuthorship } from "@/schemas/service-history";
import { AsyncOptionSelect } from "@/components/shared/async-option-select";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { ApiError } from "@/lib/api-client";

export function HistoryTime({
  value,
  relative = false,
  expanded = false,
}: {
  readonly value: string;
  readonly relative?: boolean;
  readonly expanded?: boolean;
}) {
  const date = new Date(value);
  const exact = date.toLocaleString(undefined, {
    dateStyle: "full",
    timeStyle: "long",
  });
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <time
          dateTime={value}
          tabIndex={0}
          aria-label={exact}
          className="rounded-sm font-mono text-[11px] focus-visible:outline focus-visible:outline-2 focus-visible:outline-ring"
        >
          {expanded
            ? exact
            : relative
              ? formatRelativeTime(value)
              : date.toLocaleDateString(undefined, {
                  day: "numeric",
                  month: "short",
                  year: "numeric",
                })}
        </time>
      </TooltipTrigger>
      <TooltipContent>{exact}</TooltipContent>
    </Tooltip>
  );
}

function actorName(
  actor: NonNullable<ServiceAuthorship["created_by"]>["actor"],
): string {
  const kind = (
    {
      api_key: "API key",
      service_account: "Service account",
      app: "Application",
      system: "System",
    } as Record<string, string>
  )[actor.kind];
  return kind ? `${actor.name} (${kind})` : actor.name;
}

export function ServiceAuthorshipFooter({
  authorship,
  className,
}: {
  readonly authorship?: ServiceAuthorship | null;
  readonly className?: string;
}) {
  if (!authorship) return null;
  return (
    <div
      className={cn(
        "mt-3 flex min-w-0 flex-col items-end gap-1 text-right text-[11px] text-muted-foreground",
        className,
      )}
      aria-label="Service authorship"
    >
      <p className="max-w-full break-words">
        {authorship.created_by ? (
          <>
            Created by {actorName(authorship.created_by.actor)} ·{" "}
            <HistoryTime value={authorship.created_by.at} />
          </>
        ) : (
          "Creator not recorded"
        )}
      </p>
      <p className="max-w-full break-words">
        {authorship.last_change ? (
          <>
            Last edited by {actorName(authorship.last_change.actor)} ·{" "}
            <HistoryTime value={authorship.last_change.at} relative />
          </>
        ) : authorship.created_by ? (
          "No edits since creation"
        ) : (
          "Earlier edits not recorded"
        )}
      </p>
    </div>
  );
}

function safeValue(field: string, value: unknown): string {
  if (value === null || value === undefined) return "Not recorded";
  if (typeof value === "boolean") return value ? "Yes" : "No";
  if (typeof value === "number" || typeof value === "string")
    return String(value);
  if (
    field === "token_scopes" &&
    Array.isArray(value) &&
    value.every((item) => typeof item === "string")
  )
    return value.length ? value.join(", ") : "No reviewed scopes";
  if (
    (field === "default_request_headers" || field === "ws_frame_injections") &&
    typeof value === "object" &&
    !Array.isArray(value)
  ) {
    const metadata = value as Record<string, unknown>;
    const items =
      metadata[field === "default_request_headers" ? "names" : "directions"];
    if (
      typeof metadata.count === "number" &&
      Array.isArray(items) &&
      items.every((item) => typeof item === "string")
    ) {
      const noun = field === "default_request_headers" ? "header" : "rule";
      return `${metadata.count} ${noun}${metadata.count === 1 ? "" : "s"}${items.length ? ` (${items.join(", ")})` : ""}`;
    }
  }
  return "Changed";
}

export function ServiceHistory({ serviceId }: { readonly serviceId: string }) {
  const [actions, setActions] = useState<string[]>([]);
  const query = useServiceHistory(serviceId, actions);
  // An authorization or context failure must immediately hide previously cached rows.
  const pages = query.isError ? undefined : query.data?.pages;
  const first = pages?.[0];
  const groups = pages?.flatMap((page) => page.groups) ?? [];
  return (
    <section className="space-y-4" aria-label="Service history">
      <div className="max-w-lg">
        <AsyncOptionSelect
          optionSet="service-history-action"
          context={{ kind: "service-history" }}
          label="Filter history actions"
          value={actions}
          onChange={setActions}
        />
      </div>
      {query.isPending && (
        <p role="status" className="text-[12px] text-muted-foreground">
          Loading service history…
        </p>
      )}
      {query.isError && (
        <div role="alert" className="space-y-2 text-[12px]">
          <p>
            {query.error instanceof ApiError &&
            [403, 404].includes(query.error.status)
              ? "Service history is unavailable. Personal owners and currently permitted organization admins can view it."
              : "Service history could not be loaded."}
          </p>
          <Button
            size="sm"
            variant="outline"
            onClick={() => void query.refetch()}
          >
            Retry history
          </Button>
        </div>
      )}
      {first?.deleted && (
        <p className="text-[12px] text-muted-foreground">
          This service was deleted. Its recorded history is retained.
        </p>
      )}
      {first?.legacy && (
        <p className="text-[12px] text-muted-foreground">
          Creator and earlier edits may not have been recorded.
          {first.tracked_since && (
            <>
              {" "}
              Reliable tracking began{" "}
              <HistoryTime value={first.tracked_since} />.
            </>
          )}
        </p>
      )}
      {first && groups.length === 0 && (
        <p className="text-[12px] text-muted-foreground">
          {actions.length
            ? "No changes match these filters."
            : "No recorded changes yet."}
        </p>
      )}
      <ol className="space-y-3">
        {groups.map((group) => {
          const firstEvent = group.events[0];
          if (!firstEvent) return null;
          const lastEvent = group.events[group.events.length - 1]!;
          const summaryEvent =
            group.events.find((event) => event.action === "service.deleted") ??
            group.events.find((event) => event.action === "service.created") ??
            lastEvent;
          return (
            <li key={group.id}>
              <details className="rounded-xl border border-border/50 bg-card p-4">
                <summary className="cursor-pointer text-[12px] focus-visible:outline focus-visible:outline-2 focus-visible:outline-ring">
                  <span className="font-medium">
                    {summaryEvent.action_label || "Service updated"}
                  </span>
                  <span className="ml-2 text-muted-foreground">
                    {actorName(summaryEvent.actor)} ·{" "}
                    <HistoryTime value={lastEvent.committed_at} />
                    {group.events.length > 1 &&
                      ` · ${group.events.length} changes`}
                  </span>
                </summary>
                <div className="mt-3 space-y-4">
                  {group.events.map((event) => (
                    <div key={event.id} className="space-y-2 text-[12px]">
                      <p>
                        {event.action_label || "Service updated"} ·{" "}
                        {actorName(event.actor)} ·{" "}
                        <HistoryTime value={event.committed_at} expanded />
                      </p>
                      {event.actor.api_key_id && (
                        <p className="text-muted-foreground">
                          Authenticated API key
                        </p>
                      )}
                      {event.actor.app_id && (
                        <p className="break-all text-muted-foreground">
                          Application {event.actor.app_id}
                        </p>
                      )}
                      <dl className="space-y-1">
                        {event.changes.map((change) => (
                          <div
                            key={change.field}
                            className="flex flex-wrap gap-x-2"
                          >
                            <dt className="font-medium">
                              {change.label || "Service setting"}
                            </dt>
                            <dd className="break-words text-muted-foreground">
                              {JSON.stringify(change.before ?? null) ===
                              JSON.stringify(change.after ?? null)
                                ? "Changed; values omitted"
                                : `${safeValue(change.field, change.before)} → ${safeValue(change.field, change.after)}`}
                            </dd>
                          </div>
                        ))}
                      </dl>
                      {event.additional_changes && (
                        <p className="text-muted-foreground">
                          Other service settings updated. Additional change
                          details are unavailable.
                        </p>
                      )}
                      {event.audit_status === "mismatch" && (
                        <p role="alert" className="text-destructive">
                          This record differs from its audit mirror.
                        </p>
                      )}
                    </div>
                  ))}
                </div>
              </details>
            </li>
          );
        })}
      </ol>
      {query.hasNextPage && !query.isError && (
        <Button
          variant="outline"
          isLoading={query.isFetchingNextPage}
          onClick={() => void query.fetchNextPage()}
        >
          Load older changes
        </Button>
      )}
    </section>
  );
}

export function ArchivedServiceHistory() {
  const [open, setOpen] = useState(false);
  return (
    <section className="mt-6 space-y-3" aria-label="Deleted service history">
      <Button
        variant="outline"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
      >
        Deleted service history
      </Button>
      {open && <ArchiveList />}
    </section>
  );
}
function ArchiveList() {
  const query = useArchivedServiceHistory();
  const rows = query.isError
    ? []
    : (query.data?.pages.flatMap((page) => page.services) ?? []);
  return (
    <div className="space-y-3 rounded-xl border border-border/50 p-4">
      <p className="text-xs text-muted-foreground">
        Recorded history of deleted services you currently have permission to
        manage.
      </p>
      {query.isPending && <p role="status">Loading deleted service history…</p>}
      {query.isError && (
        <div role="alert">
          <p>Deleted service history could not be loaded.</p>
          <Button variant="outline" onClick={() => void query.refetch()}>
            Retry archived history
          </Button>
        </div>
      )}
      {query.isSuccess && !rows.length && (
        <p className="text-xs text-muted-foreground">
          No deleted service history is available.
        </p>
      )}
      <ul className="space-y-2">
        {rows.map((row) => (
          <li
            key={row.service_id}
            className="flex flex-wrap justify-between gap-2 text-xs"
          >
            <Link
              to="/keys/$keyId"
              params={{ keyId: row.service_id }}
              className="break-all text-primary underline underline-offset-2"
            >
              {row.service_slug} · View history
            </Link>
            <span className="text-muted-foreground">
              Last change <HistoryTime value={row.last_changed_at} />
            </span>
          </li>
        ))}
      </ul>
      {query.hasNextPage && !query.isError && (
        <Button
          variant="outline"
          isLoading={query.isFetchingNextPage}
          onClick={() => void query.fetchNextPage()}
        >
          Load older deleted services
        </Button>
      )}
    </div>
  );
}
