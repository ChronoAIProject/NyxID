import { useEffect, useId, useMemo, useRef, useState } from "react";
import { Check, ChevronsUpDown } from "lucide-react";
import { ErrorBanner } from "@/components/shared/error-banner";
import { ServiceIcon } from "@/components/service-icon";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import type { PoolCandidate } from "@/schemas/pools";
import { bindingLabel, message, protocolLabel, reason } from "./pool-labels";

type Props = {
  rows: PoolCandidate[];
  selectedIds: string[];
  search: string;
  onSearch: (search: string) => void;
  onToggle: (row: PoolCandidate) => void;
  isLoading: boolean;
  isSearching: boolean;
  isCheckingCompatibility?: boolean;
  isRefreshing?: boolean;
  isError: boolean;
  error: unknown;
  onRetry: () => void;
  hasNextPage: boolean;
  isFetchingNextPage: boolean;
  onLoadMore: () => void;
};

export function PoolConnectionPicker({
  rows,
  selectedIds,
  search,
  onSearch,
  onToggle,
  isLoading,
  isSearching,
  isCheckingCompatibility = false,
  isRefreshing = false,
  isError,
  error,
  onRetry,
  hasNextPage,
  isFetchingNextPage,
  onLoadMore,
}: Props) {
  const [open, setOpen] = useState(false);
  const [activeId, setActiveId] = useState<string | null>(null);
  const id = useId();
  const listId = `${id}-list`;
  const input = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const loadMoreButton = useRef<HTMLButtonElement>(null);
  const restoreLoadMoreFocus = useRef(false);
  useEffect(() => {
    if (isFetchingNextPage || !restoreLoadMoreFocus.current) return;
    restoreLoadMoreFocus.current = false;
    if (document.activeElement === document.body)
      (hasNextPage ? loadMoreButton.current : input.current)?.focus();
  }, [isFetchingNextPage, hasNextPage, rows]);
  const groups = useMemo(() => {
    const grouped = new Map<
      string,
      {
        key: string;
        name: string;
        slug: string | null;
        rows: PoolCandidate[];
      }
    >();
    const seen = new Set<string>();
    for (const row of rows) {
      if (seen.has(row.user_service_id)) continue;
      seen.add(row.user_service_id);
      const key = row.catalog_service_id ?? "custom";
      const existing = grouped.get(key);
      if (existing) {
        existing.rows.push(row);
        continue;
      }
      grouped.set(key, {
        key,
        name:
          row.group_name?.trim() ||
          (row.catalog_service_id ? "Catalog service" : "Custom connections"),
        slug: row.group_slug ?? null,
        rows: [row],
      });
    }
    return [...grouped.values()];
  }, [rows]);
  const displayRows = useMemo(
    () => groups.flatMap((group) => group.rows),
    [groups],
  );
  const displayIndex = useMemo(
    () =>
      new Map(displayRows.map((row, index) => [row.user_service_id, index])),
    [displayRows],
  );
  const activeIndex = displayRows.findIndex(
    (row) => row.user_service_id === activeId,
  );
  const busy = isLoading || isSearching;
  useEffect(() => {
    if (activeIndex >= 0)
      list.current
        ?.querySelector(`[data-index="${activeIndex}"]`)
        ?.scrollIntoView?.({ block: "nearest" });
  }, [activeIndex]);
  function disabled(row: PoolCandidate) {
    return (
      !selectedIds.includes(row.user_service_id) &&
      (isCheckingCompatibility ||
        selectedIds.length >= 50 ||
        (!row.eligible && row.reason !== "compatibility_declaration_required"))
    );
  }
  function choose(row: PoolCandidate) {
    if (!disabled(row)) onToggle(row);
    input.current?.focus();
  }
  function navigate(direction: number) {
    if (!displayRows.length) return;
    const next =
      activeIndex < 0
        ? direction > 0
          ? 0
          : displayRows.length - 1
        : (activeIndex + direction + displayRows.length) % displayRows.length;
    setActiveId(displayRows[next]!.user_service_id);
  }
  return (
    <div className="space-y-2">
      <Popover open={open} onOpenChange={setOpen}>
        <PopoverTrigger asChild>
          <Button
            type="button"
            variant="outline"
            className="w-full justify-between gap-2 text-left"
            aria-label="Choose connections"
          >
            <span className="min-w-0 truncate">
              {selectedIds.length
                ? `${selectedIds.length} ${selectedIds.length === 1 ? "connection" : "connections"} selected`
                : "Search and select connections"}
            </span>
            <ChevronsUpDown className="size-4 shrink-0" aria-hidden="true" />
          </Button>
        </PopoverTrigger>
        <PopoverContent
          align="start"
          collisionPadding={12}
          aria-label="Choose pool connections"
          className="flex max-h-[min(28rem,calc(50dvh-2rem))] w-[var(--radix-popover-trigger-width)] max-w-[calc(100vw-1.5rem)] flex-col gap-2 p-2 [@media(max-height:500px)]:gap-1 [@media(max-height:500px)]:p-1.5 data-[state=closed]:hidden data-[state=closed]:animate-none"
          onOpenAutoFocus={(event) => {
            event.preventDefault();
            input.current?.focus();
          }}
          onEscapeKeyDown={(event) => {
            event.preventDefault();
            event.stopPropagation();
            setOpen(false);
          }}
        >
          <Input
            ref={input}
            role="combobox"
            aria-label="Search candidate services"
            aria-expanded={open}
            aria-controls={listId}
            aria-autocomplete="list"
            aria-haspopup="listbox"
            aria-activedescendant={
              activeIndex >= 0 ? `${id}-option-${activeIndex}` : undefined
            }
            autoComplete="off"
            placeholder="Search your connections"
            className="shrink-0 focus-visible:border-primary focus-visible:ring-1 focus-visible:ring-primary/40"
            value={search}
            onChange={(event) => {
              setActiveId(null);
              onSearch(event.target.value);
            }}
            onKeyDown={(event) => {
              if (event.nativeEvent.isComposing) return;
              if (event.key === "ArrowDown" || event.key === "ArrowUp") {
                event.preventDefault();
                navigate(event.key === "ArrowDown" ? 1 : -1);
              } else if (event.key === "Enter") {
                event.preventDefault();
                if (activeIndex >= 0) choose(displayRows[activeIndex]!);
              }
            }}
          />
          <p
            aria-live="polite"
            className="px-1 text-11 text-muted-foreground [@media(max-height:500px)]:sr-only"
          >
            {isCheckingCompatibility
              ? isError
                ? "Compatibility check failed."
                : "Checking compatibility…"
              : "Select multiple connections. Select again to remove."}
          </p>
          <div className="min-h-0 overflow-y-auto overscroll-contain">
            {isError && (
              <div role="alert">
                <ErrorBanner message={message(error)} onRetry={onRetry} />
              </div>
            )}
            {busy && (
              <p
                role="status"
                className="p-2 text-12 text-muted-foreground"
              >
                {isLoading ? "Loading connections…" : "Searching connections…"}
              </p>
            )}
            <div
              ref={list}
              id={listId}
              role="listbox"
              aria-label="Connections"
              aria-multiselectable="true"
              aria-busy={busy || isCheckingCompatibility}
            >
              {groups.map((group) => (
                <div
                  key={group.key}
                  role="group"
                  aria-label={`${group.name}${group.slug ? ` (${group.slug})` : ""} (${group.rows.length} loaded)`}
                  className="border-b border-border/40 last:border-b-0"
                >
                  <div className="flex min-w-0 items-center gap-2 px-2 pb-1 pt-2 text-11 font-semibold text-muted-foreground">
                    <ServiceIcon slug={group.slug} size="xs" />
                    <span className="min-w-0 flex-1 break-words">
                      {group.name}
                    </span>
                    {group.slug && (
                      <span className="max-w-[35%] shrink-0 break-all font-normal">
                        {group.slug}
                      </span>
                    )}
                    <span className="shrink-0 font-normal">
                      {group.rows.length} loaded
                    </span>
                  </div>
                  {group.rows.map((row) => {
                    const index = displayIndex.get(row.user_service_id) ?? 0;
                    const selected = selectedIds.includes(row.user_service_id);
                    const unavailable = disabled(row);
                    return (
                      <div
                        key={row.user_service_id}
                        id={`${id}-option-${index}`}
                        data-index={index}
                        role="option"
                        aria-label={row.name || row.slug}
                        aria-describedby={`${id}-description-${index}`}
                        aria-selected={selected}
                        aria-disabled={unavailable}
                        className={`flex min-w-0 items-start gap-2 rounded-lg p-2 text-left ${activeIndex === index ? "bg-accent ring-1 ring-inset ring-primary/40" : "hover:bg-accent/50"} ${unavailable ? "cursor-not-allowed" : "cursor-pointer"}`}
                        onPointerDown={(event) => event.preventDefault()}
                        onClick={() => choose(row)}
                      >
                        <span
                          aria-hidden="true"
                          className={`mt-0.5 flex size-4 shrink-0 items-center justify-center rounded border ${unavailable ? "opacity-50" : ""} ${selected ? "border-primary bg-primary text-primary-foreground" : "border-muted-foreground/50"}`}
                        >
                          {selected && <Check className="size-3" />}
                        </span>
                        <div className="min-w-0 flex-1">
                          <p className="break-words text-12 font-medium">
                            {row.name || row.slug}
                          </p>
                          <div
                            id={`${id}-description-${index}`}
                            className="break-words text-11 text-muted-foreground"
                          >
                            <p>
                              {row.slug} ·{" "}
                              {bindingLabel(row.credential_binding)}
                              {row.protocol
                                ? ` · ${protocolLabel(row.protocol)}`
                                : ""}
                            </p>
                            {row.reason && (
                              <p className="mt-0.5">{reason(row)}</p>
                            )}
                          </div>
                        </div>
                      </div>
                    );
                  })}
                </div>
              ))}
            </div>
            {!busy && !isError && rows.length === 0 && (
              <p
                role="status"
                className="p-2 text-12 text-muted-foreground"
              >
                {search
                  ? "No connections match this search."
                  : "No connections yet. Connect a service in the Services tab first, then return here."}
              </p>
            )}
            {hasNextPage && (
              <Button
                ref={loadMoreButton}
                type="button"
                className="mt-2 w-full"
                isLoading={isFetchingNextPage}
                disabled={
                  isFetchingNextPage || isRefreshing || isCheckingCompatibility
                }
                onClick={(event) => {
                  restoreLoadMoreFocus.current =
                    document.activeElement === event.currentTarget;
                  void onLoadMore();
                }}
              >
                Load more connections
              </Button>
            )}
          </div>
          <div className="flex shrink-0 items-center justify-between gap-2 border-t border-border px-1 pt-2">
            <p role="status" className="text-11 text-muted-foreground">
              {selectedIds.length} of 50 selected
            </p>
            <Button type="button" size="sm" onClick={() => setOpen(false)}>
              Done
            </Button>
          </div>
          {selectedIds.length >= 50 && (
            <p className="text-11 text-muted-foreground">
              A pool supports up to 50 connections. Remove one to choose
              another.
            </p>
          )}
        </PopoverContent>
      </Popover>
    </div>
  );
}
