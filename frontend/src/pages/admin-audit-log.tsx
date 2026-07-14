import { useCallback, useEffect, useRef, useState } from "react";
import { useNavigate, useSearch } from "@tanstack/react-router";
import { useAdminAuditLog } from "@/hooks/use-admin";
import { cn, formatDateTime } from "@/lib/utils";
import type {
  AdminAuditLogEntry,
  AdminAuditLogFilterKey,
  AdminAuditLogFilterSelections,
  AdminAuditLogListParams,
  AdminAuditLogPerPage,
  AdminAuditLogSearchFieldKey,
  AdminAuditLogSearchState,
  AdminAuditLogSort,
  AdminAuditLogSortField,
} from "@/types/admin";
import type { DataTableSearchApplyMode } from "@/types/data-table";
import { PageHeader } from "@/components/shared/page-header";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  AlertTriangle,
  ChevronLeft,
  ChevronRight,
  ClipboardList,
  Loader2,
  RotateCcw,
} from "lucide-react";
import { toast } from "sonner";
import {
  ADMIN_AUDIT_LOG_DEFAULT_PER_PAGE,
  ADMIN_AUDIT_LOG_PER_PAGE_OPTIONS,
  ADMIN_AUDIT_LOG_SORTS,
  adminAuditLogCustomFilterPatch,
  adminAuditLogFilterPatch,
  getAdminAuditLogCustomValues,
  getAdminAuditLogFilterFields,
  getAdminAuditLogFilterValues,
  getAdminAuditLogSearchFields,
  getAdminAuditLogSearchFilters,
  getAppliedAdminAuditLogFilters,
  isAdminAuditLogDateFilterValid,
  normalizeAdminAuditLogSearch,
  parseAdminAuditLogSearchFilters,
  updateAdminAuditLogSearchFilters,
} from "@/lib/admin-audit-log";
import {
  DataTableControls,
  DataTableFilterChips,
  DataTableFilterPopover,
  DataTableSearch,
} from "@/components/data-table/data-table-controls";
import {
  DATA_TABLE_CLASS_NAME,
  DataTableBadgeCell,
  DataTableCellShell,
  DataTableColumnHeader,
  nextDataTableSort,
  useDataTableColumns,
  type DataTableColumn,
} from "@/components/data-table/data-table-columns";

const DEFAULT_SORT: AdminAuditLogSort = "-created_at";

/** Bump the version suffix when a change makes stored layouts unreadable. */
const COLUMN_PREFERENCES_KEY = "nyxid.table.admin-audit-log.columns.v1";

const AUDIT_LOG_COLUMNS: readonly DataTableColumn<AdminAuditLogSortField>[] = [
  {
    field: "created_at",
    label: "Created",
    defaultWidth: 190,
    cellClassName: "whitespace-nowrap text-sm text-muted-foreground",
  },
  {
    field: "event_type",
    label: "Event",
    defaultWidth: 240,
  },
  {
    field: "api_key_name",
    label: "Agent",
    defaultWidth: 180,
  },
  {
    field: "status",
    label: "Status",
    // Wide enough that the label still reads in full alongside the header's
    // drag, sort, and freeze controls.
    defaultWidth: 150,
  },
  {
    field: "user_id",
    label: "User ID",
    defaultWidth: 300,
    cellClassName: "font-mono text-[11px] text-muted-foreground",
  },
  {
    field: "api_key_id",
    label: "API Key ID",
    defaultWidth: 300,
    cellClassName: "font-mono text-[11px] text-muted-foreground",
  },
  {
    field: "ip_address",
    label: "IP",
    defaultWidth: 150,
    cellClassName: "font-mono text-[11px]",
  },
  {
    field: "user_agent",
    label: "User Agent",
    defaultWidth: 280,
    cellClassName: "text-muted-foreground",
  },
];

/**
 * Direction a column takes on its first click. Identity and free-text columns
 * read best A-Z; time and status lead with the most recent / most severe.
 */
const DEFAULT_COLUMN_SORT: Record<AdminAuditLogSortField, AdminAuditLogSort> = {
  created_at: "-created_at",
  event_type: "event_type",
  api_key_name: "api_key_name",
  status: "-status",
  user_id: "user_id",
  api_key_id: "api_key_id",
  ip_address: "ip_address",
  user_agent: "user_agent",
};

type AuditLogSearchEditTarget =
  | { readonly kind: "all"; readonly value: string }
  | {
      readonly kind: "field";
      readonly field: AdminAuditLogSearchFieldKey;
      readonly value: string;
    };

function responseStatus(entry: AdminAuditLogEntry): number | null {
  const value = entry.event_data?.response_status;
  return typeof value === "number" ? value : null;
}

function statusVariant(
  status: number | null,
): "default" | "secondary" | "destructive" {
  if (status === null) return "secondary";
  if (status >= 500) return "destructive";
  if (status >= 400) return "secondary";
  if (status >= 200) return "default";
  return "secondary";
}

export function AdminAuditLogPage() {
  const navigate = useNavigate();
  const routeSearch = useSearch({
    strict: false,
  }) as AdminAuditLogSearchState;
  const {
    order: columnOrder,
    announcement: columnAnnouncement,
    resizingColumn,
    tableRef,
    tableStyle,
    columnStyle,
    isDefaultLayout: isDefaultColumnLayout,
    resetLayout: resetColumnLayout,
    headerProps: columnHeaderProps,
    cellProps: columnCellProps,
  } = useDataTableColumns(AUDIT_LOG_COLUMNS, COLUMN_PREFERENCES_KEY);
  const appliedSearch = routeSearch.search ?? "";
  const [searchDraft, setSearchDraft] = useState("");
  const [selectedSearchField, setSelectedSearchField] =
    useState<AdminAuditLogSearchFieldKey | null>(null);
  const [editingSearch, setEditingSearch] =
    useState<AuditLogSearchEditTarget | null>(null);
  const previousSearchRouteRef = useRef(
    `${routeSearch.search ?? ""}\u0000${routeSearch.search_filters ?? ""}`,
  );
  const [filterPopoverOpen, setFilterPopoverOpen] = useState(false);
  const [selectedFilterKey, setSelectedFilterKey] =
    useState<AdminAuditLogFilterKey>("event_type");
  const searchInputRef = useRef<HTMLInputElement>(null);

  const listParams: AdminAuditLogListParams = {
    page: routeSearch.page ?? 1,
    per_page: routeSearch.per_page ?? ADMIN_AUDIT_LOG_DEFAULT_PER_PAGE,
    search: routeSearch.search,
    search_filters: routeSearch.search_filters,
    custom_filters: routeSearch.custom_filters,
    event_type: routeSearch.event_type,
    status: routeSearch.status,
    actor: routeSearch.actor,
    created_dates: routeSearch.created_dates,
    created_from: routeSearch.created_from,
    created_to: routeSearch.created_to,
    sort: routeSearch.sort ?? DEFAULT_SORT,
  };

  const { data, isLoading, isFetching, isPlaceholderData, error, refetch } =
    useAdminAuditLog(listParams);

  const entries = data?.entries ?? [];
  const total = data?.total ?? 0;
  const totalPages = Math.max(1, Math.ceil(total / listParams.per_page));
  const displayedPage = data?.page ?? listParams.page;
  const displayedPerPage = data?.per_page ?? listParams.per_page;
  const displayedTotalPages = Math.max(1, Math.ceil(total / displayedPerPage));
  const filterOptions = data?.filter_options;
  const filterFields = getAdminAuditLogFilterFields(filterOptions);
  const searchFields = getAdminAuditLogSearchFields(filterOptions);
  const appliedSearchFilters = getAdminAuditLogSearchFilters(listParams);
  const appliedStructuredFilters = getAppliedAdminAuditLogFilters(
    filterFields,
    listParams,
  );
  const filterSelections: AdminAuditLogFilterSelections = {};
  const customFilterSelections: AdminAuditLogFilterSelections = {};
  for (const field of filterFields) {
    filterSelections[field.key] = getAdminAuditLogFilterValues(
      listParams,
      field.key,
    );
    customFilterSelections[field.key] = getAdminAuditLogCustomValues(
      listParams,
      field.key,
    );
  }
  const availableSorts = new Set(filterOptions?.sorts ?? ADMIN_AUDIT_LOG_SORTS);
  const hasActiveFilters = Boolean(
    listParams.search ||
      appliedSearchFilters.length > 0 ||
      appliedStructuredFilters.length > 0,
  );

  const updateListSearch = useCallback(
    (patch: Partial<AdminAuditLogSearchState>, replace = false) => {
      void navigate({
        to: "/admin/audit-log",
        search: (previous) =>
          normalizeAdminAuditLogSearch({ ...previous, ...patch }),
        replace,
      });
    },
    [navigate],
  );

  useEffect(() => {
    if (!isPlaceholderData && total > 0 && listParams.page > totalPages) {
      updateListSearch({ page: totalPages }, true);
    }
  }, [isPlaceholderData, listParams.page, total, totalPages, updateListSearch]);

  useEffect(() => {
    const nextRoute = `${routeSearch.search ?? ""}\u0000${
      routeSearch.search_filters ?? ""
    }`;
    if (nextRoute === previousSearchRouteRef.current) return;
    previousSearchRouteRef.current = nextRoute;
    const resetTimer = window.setTimeout(() => {
      setSearchDraft("");
      setEditingSearch(null);
    }, 0);
    return () => window.clearTimeout(resetTimer);
  }, [routeSearch.search, routeSearch.search_filters]);

  function clearSearchAndFilters() {
    updateListSearch({
      page: undefined,
      search: undefined,
      search_filters: undefined,
      custom_filters: undefined,
      event_type: undefined,
      status: undefined,
      actor: undefined,
      created_dates: undefined,
      created_from: undefined,
      created_to: undefined,
    });
  }

  function focusSearchInput() {
    window.requestAnimationFrame(() => {
      searchInputRef.current?.focus();
      searchInputRef.current?.select();
    });
  }

  function cancelSearchDraft() {
    setSearchDraft("");
    setEditingSearch(null);
  }

  function removeScopedSearchValue(
    field: AdminAuditLogSearchFieldKey,
    value: string,
    navigate = true,
  ): string | undefined {
    const current = parseAdminAuditLogSearchFilters(listParams.search_filters);
    const values =
      current
        ?.find((group) => group.field === field)
        ?.values.filter(
          (item) => item.toLocaleLowerCase() !== value.toLocaleLowerCase(),
        ) ?? [];
    const next = updateAdminAuditLogSearchFilters(
      listParams.search_filters,
      field,
      values.length > 0 ? values : undefined,
    );
    if (navigate) {
      updateListSearch({ page: undefined, search_filters: next });
      if (
        editingSearch?.kind === "field" &&
        editingSearch.field === field &&
        editingSearch.value.toLocaleLowerCase() === value.toLocaleLowerCase()
      ) {
        cancelSearchDraft();
      }
    }
    return next;
  }

  function applySearchDraft(mode: DataTableSearchApplyMode) {
    const term = searchDraft.trim();
    if (!term) return;

    let nextLegacySearch = listParams.search;
    let nextScopedSearch = listParams.search_filters;
    if (editingSearch?.kind === "all") {
      nextLegacySearch = undefined;
    } else if (editingSearch?.kind === "field") {
      const current = parseAdminAuditLogSearchFilters(nextScopedSearch);
      const remaining =
        current
          ?.find((group) => group.field === editingSearch.field)
          ?.values.filter(
            (value) =>
              value.toLocaleLowerCase() !==
              editingSearch.value.toLocaleLowerCase(),
          ) ?? [];
      nextScopedSearch = updateAdminAuditLogSearchFilters(
        nextScopedSearch,
        editingSearch.field,
        remaining.length > 0 ? remaining : undefined,
      );
    }

    if (selectedSearchField === null) {
      nextLegacySearch = term;
    } else {
      const current = parseAdminAuditLogSearchFilters(nextScopedSearch) ?? [];
      const values =
        current.find((group) => group.field === selectedSearchField)?.values ??
        [];
      const duplicate = values.some(
        (value) => value.toLocaleLowerCase() === term.toLocaleLowerCase(),
      );
      const nextValues = duplicate ? values : [...values, term];
      const encoded = updateAdminAuditLogSearchFilters(
        nextScopedSearch,
        selectedSearchField,
        nextValues,
      );
      if (!encoded) {
        toast.error("Search supports up to 8 values per field and 32 overall");
        return;
      }
      nextScopedSearch = encoded;
    }

    updateListSearch({
      page: undefined,
      search: nextLegacySearch,
      search_filters: nextScopedSearch,
    });
    cancelSearchDraft();
    if (mode === "submit") {
      setSelectedSearchField(null);
      focusSearchInput();
    }
  }

  function editLegacySearch() {
    setSelectedSearchField(null);
    setSearchDraft(appliedSearch);
    setEditingSearch({ kind: "all", value: appliedSearch });
    focusSearchInput();
  }

  function editScopedSearchValue(
    field: AdminAuditLogSearchFieldKey,
    value: string,
  ) {
    setSelectedSearchField(field);
    setSearchDraft(value);
    setEditingSearch({ kind: "field", field, value });
    focusSearchInput();
  }

  function applyStructuredFilters(
    selections: AdminAuditLogFilterSelections,
    customSelections: AdminAuditLogFilterSelections,
  ) {
    const patch: Partial<AdminAuditLogSearchState> = { page: undefined };
    for (const field of filterFields) {
      const fieldPatch = adminAuditLogFilterPatch(
        field.key,
        selections[field.key],
      );
      if (!fieldPatch) return;
      Object.assign(patch, fieldPatch);
    }
    Object.assign(patch, adminAuditLogCustomFilterPatch(customSelections));
    updateListSearch(patch);
  }

  function removeStructuredFilter(
    key: AdminAuditLogFilterKey,
    custom: boolean,
  ) {
    if (custom) {
      const remaining = { ...customFilterSelections, [key]: undefined };
      updateListSearch({
        page: undefined,
        ...adminAuditLogCustomFilterPatch(remaining),
      });
      return;
    }
    const patch = adminAuditLogFilterPatch(key, undefined);
    if (patch) updateListSearch({ page: undefined, ...patch });
  }

  function editStructuredFilter(key: AdminAuditLogFilterKey) {
    setSelectedFilterKey(key);
    setFilterPopoverOpen(true);
  }

  function updateSort(sort: AdminAuditLogSort) {
    updateListSearch({
      page: undefined,
      sort: sort === DEFAULT_SORT ? undefined : sort,
    });
  }

  function canSortColumn(field: AdminAuditLogSortField) {
    return (
      availableSorts.has(field as AdminAuditLogSort) &&
      availableSorts.has(`-${field}` as AdminAuditLogSort)
    );
  }

  function renderAuditLogCell(
    entry: AdminAuditLogEntry,
    field: AdminAuditLogSortField,
  ): React.ReactNode {
    switch (field) {
      case "created_at":
        return formatDateTime(entry.created_at);
      case "event_type":
        return (
          <span
            className="line-clamp-2 break-words text-sm font-medium text-foreground"
            title={entry.event_type}
          >
            {entry.event_type}
          </span>
        );
      case "api_key_name":
        return entry.api_key_name ? (
          <DataTableBadgeCell>
            {/* No line-clamp on the badge: clamping forces `display:-webkit-box`
                and would undo the pill's inline-flex layout. It wraps instead. */}
            <Badge
              variant="secondary"
              className="max-w-full break-all whitespace-normal"
              title={entry.api_key_name}
            >
              {entry.api_key_name}
            </Badge>
          </DataTableBadgeCell>
        ) : (
          <EmptyCell />
        );
      case "status": {
        const status = responseStatus(entry);
        return status === null ? (
          <EmptyCell />
        ) : (
          <DataTableBadgeCell>
            <Badge variant={statusVariant(status)}>{status}</Badge>
          </DataTableBadgeCell>
        );
      }
      case "user_id":
        return entry.user_id ? (
          <span className="line-clamp-2 break-all" title={entry.user_id}>
            {entry.user_id}
          </span>
        ) : (
          <EmptyCell />
        );
      case "api_key_id":
        return entry.api_key_id ? (
          <span className="line-clamp-2 break-all" title={entry.api_key_id}>
            {entry.api_key_id}
          </span>
        ) : (
          <EmptyCell />
        );
      case "ip_address":
        return entry.ip_address ? (
          <span className="line-clamp-2 break-all">{entry.ip_address}</span>
        ) : (
          <EmptyCell />
        );
      case "user_agent":
        return entry.user_agent ? (
          <span className="line-clamp-2 break-words" title={entry.user_agent}>
            {entry.user_agent}
          </span>
        ) : (
          <EmptyCell />
        );
    }
  }

  return (
    <div className="space-y-8">
      <PageHeader
        title="Audit Log"
        description="Search, filter, and sort audit activity across users and agent API keys."
      />

      <section
        className="m-0 overflow-hidden rounded-lg border border-border/60 bg-card"
        aria-label="Audit log"
      >
        <DataTableControls
          search={
            <DataTableSearch
              fields={searchFields}
              value={searchDraft}
              selectedField={selectedSearchField}
              inputRef={searchInputRef}
              ariaLabel="Search audit log"
              onValueChange={setSearchDraft}
              onFieldChange={setSelectedSearchField}
              onApply={applySearchDraft}
              onCancel={cancelSearchDraft}
            />
          }
          filter={
            <DataTableFilterPopover
              fields={filterFields}
              values={filterSelections}
              customValues={customFilterSelections}
              open={filterPopoverOpen}
              selectedKey={selectedFilterKey}
              activeCount={
                appliedStructuredFilters.length +
                appliedSearchFilters.length +
                (appliedSearch ? 1 : 0)
              }
              validateValues={(field, values) =>
                field.value_type !== "date" ||
                isAdminAuditLogDateFilterValid(values)
              }
              onOpenChange={setFilterPopoverOpen}
              onSelectField={setSelectedFilterKey}
              onApply={applyStructuredFilters}
            />
          }
          status={
            isFetching && !isLoading ? (
              <div
                role="status"
                aria-live="polite"
                className="flex items-center gap-1.5 text-xs text-muted-foreground"
              >
                <Loader2
                  className="h-3.5 w-3.5 animate-spin"
                  aria-hidden="true"
                />
                <span>Updating results</span>
              </div>
            ) : null
          }
          chips={
            <DataTableFilterChips
              search={appliedSearch}
              searchFields={searchFields}
              searchFilters={appliedSearchFilters}
              filters={appliedStructuredFilters}
              onEditSearch={editLegacySearch}
              onRemoveSearch={() =>
                updateListSearch({ page: undefined, search: undefined })
              }
              onEditSearchValue={editScopedSearchValue}
              onRemoveSearchValue={removeScopedSearchValue}
              onEdit={editStructuredFilter}
              onRemove={removeStructuredFilter}
              onClear={clearSearchAndFilters}
            />
          }
        />

        {error && data && (
          <div
            role="alert"
            className="flex flex-col gap-3 border-b border-destructive/25 bg-destructive/5 px-3 py-2.5 text-sm sm:flex-row sm:items-center sm:justify-between"
          >
            <div className="flex items-center gap-2 text-destructive">
              <AlertTriangle className="h-4 w-4 shrink-0" aria-hidden="true" />
              <span>
                Results may be out of date because the refresh failed.
              </span>
            </div>
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={isFetching}
              onClick={() => void refetch()}
            >
              Retry
            </Button>
          </div>
        )}

        {isLoading || (isPlaceholderData && entries.length === 0) ? (
          <div
            className="space-y-2 p-3"
            role="status"
            aria-live="polite"
            aria-label={
              isLoading ? "Loading audit log" : "Updating audit log"
            }
          >
            {Array.from({ length: 6 }).map((_, i) => (
              <Skeleton
                key={`audit-log-skel-${String(i)}`}
                className="h-12 w-full"
              />
            ))}
          </div>
        ) : error && !data ? (
          <div
            role="alert"
            className="flex min-h-56 flex-col items-center justify-center gap-2 bg-destructive/5 px-4 py-12 text-center"
          >
            <AlertTriangle
              className="h-8 w-8 text-destructive"
              aria-hidden="true"
            />
            <p className="text-sm font-medium text-destructive">
              Failed to load audit log
            </p>
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => void refetch()}
            >
              Retry
            </Button>
          </div>
        ) : entries.length === 0 ? (
          <div className="flex min-h-56 flex-col items-center justify-center gap-3 px-4 py-14 text-center">
            <ClipboardList className="h-10 w-10 text-muted-foreground" />
            <div>
              <p className="text-sm font-medium text-foreground">
                {hasActiveFilters
                  ? "No events match these filters"
                  : "No audit events"}
              </p>
              <p className="text-xs text-muted-foreground">
                {hasActiveFilters
                  ? "Adjust or clear the current filters."
                  : "Audit activity will appear here."}
              </p>
              {hasActiveFilters && (
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  className="mt-3"
                  onClick={clearSearchAndFilters}
                >
                  Clear search and filters
                </Button>
              )}
            </div>
          </div>
        ) : (
          <>
            <div
              className={cn(
                "transition-opacity motion-reduce:transition-none",
                isPlaceholderData && "opacity-60",
                // Hold the resize cursor for the whole drag, wherever it lands.
                resizingColumn && "cursor-col-resize select-none",
              )}
              aria-busy={isFetching}
            >
              <p className="sr-only" aria-live="polite" aria-atomic="true">
                {columnAnnouncement}
              </p>
              <Table
                ref={tableRef}
                containerClassName="overscroll-x-none"
                className={DATA_TABLE_CLASS_NAME}
                style={tableStyle}
              >
                <colgroup>
                  {columnOrder.map((field) => (
                    <col key={field} style={columnStyle(field)} />
                  ))}
                </colgroup>
                <TableHeader>
                  <TableRow>
                    {columnOrder.map((field) => (
                      <DataTableColumnHeader
                        key={field}
                        {...columnHeaderProps(field)}
                        sort={listParams.sort}
                        nextSort={nextDataTableSort(
                          listParams.sort,
                          field,
                          DEFAULT_COLUMN_SORT,
                        )}
                        disabled={!canSortColumn(field)}
                        onSort={updateSort}
                      />
                    ))}
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {entries.map((entry) => (
                    <TableRow
                      key={entry.id}
                      className="group/row hover:bg-muted/45"
                    >
                      {columnOrder.map((field) => (
                        <TableCell key={field} {...columnCellProps(field)}>
                          <DataTableCellShell>{renderAuditLogCell(entry, field)}</DataTableCellShell>
                        </TableCell>
                      ))}
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>

            <div className="flex flex-col gap-3 border-t border-border/60 px-3 py-3 sm:flex-row sm:items-center sm:justify-between">
              <p className="text-[11px] text-text-tertiary">
                Showing {String((displayedPage - 1) * displayedPerPage + 1)}-
                {String(Math.min(displayedPage * displayedPerPage, total))} of{" "}
                {String(total)} events
              </p>
              <div className="flex flex-wrap items-center gap-2">
                {!isDefaultColumnLayout && (
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    onClick={resetColumnLayout}
                  >
                    <RotateCcw className="mr-2 h-3.5 w-3.5" aria-hidden="true" />
                    Reset columns
                  </Button>
                )}
                <Select
                  value={String(listParams.per_page)}
                  disabled={isFetching}
                  onValueChange={(value) =>
                    updateListSearch({
                      page: undefined,
                      per_page:
                        Number(value) === ADMIN_AUDIT_LOG_DEFAULT_PER_PAGE
                          ? undefined
                          : (Number(value) as AdminAuditLogPerPage),
                    })
                  }
                >
                  <SelectTrigger
                    aria-label="Rows per page"
                    className="w-[112px]"
                  >
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {ADMIN_AUDIT_LOG_PER_PAGE_OPTIONS.map((rows) => (
                      <SelectItem key={rows} value={String(rows)}>
                        {String(rows)} rows
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <Button
                  type="button"
                  variant="outline"
                  size="icon"
                  className="h-11 w-11 md:h-8 md:w-8"
                  disabled={listParams.page <= 1 || isFetching}
                  onClick={() =>
                    updateListSearch({ page: Math.max(1, listParams.page - 1) })
                  }
                  aria-label="Previous page"
                >
                  <ChevronLeft />
                </Button>
                <span className="min-w-[84px] text-center text-[11px] text-text-tertiary">
                  Page {String(displayedPage)} of {String(displayedTotalPages)}
                </span>
                <Button
                  type="button"
                  variant="outline"
                  size="icon"
                  className="h-11 w-11 md:h-8 md:w-8"
                  disabled={listParams.page >= totalPages || isFetching}
                  onClick={() => updateListSearch({ page: listParams.page + 1 })}
                  aria-label="Next page"
                >
                  <ChevronRight />
                </Button>
              </div>
            </div>
          </>
        )}
      </section>
    </div>
  );
}

function EmptyCell() {
  return <span className="text-muted-foreground">--</span>;
}

