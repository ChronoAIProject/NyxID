import { useCallback, useEffect, useRef, useState } from "react";
import { useNavigate, useSearch } from "@tanstack/react-router";
import {
  useAdminOAuthClients,
  useBrokerSettings,
  useUpdateAdminOAuthClient,
  useUpdateBrokerSettings,
} from "@/hooks/use-admin-oauth-clients";
import { ApiError } from "@/lib/api-client";
import { cn, formatDate } from "@/lib/utils";
import { canAdminWrite } from "@/types/api";
import type {
  AdminOAuthClient,
  AdminOAuthClientFilterKey,
  AdminOAuthClientFilterSelections,
  AdminOAuthClientListParams,
  AdminOAuthClientPerPage,
  AdminOAuthClientSearchFieldKey,
  AdminOAuthClientSearchState,
  AdminOAuthClientSort,
  BrokerSettingsResponse,
  BrokerPolicyField,
  UpdateAdminOAuthClientRequest,
  UpdateBrokerSettingsRequest,
} from "@/types/admin";
import type { DataTableSearchApplyMode } from "@/types/data-table";
import type { DataTableColumnPreferences } from "@/lib/data-table-preferences";
import { useAuthStore } from "@/stores/auth-store";
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
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Switch } from "@/components/ui/switch";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  AlertTriangle,
  ArrowDown,
  ArrowUp,
  ArrowUpDown,
  ChevronLeft,
  ChevronRight,
  GripVertical,
  KeyRound,
  Loader2,
  Pin,
  PinOff,
  RotateCcw,
} from "lucide-react";
import { toast } from "sonner";
import {
  ADMIN_OAUTH_CLIENT_DEFAULT_PER_PAGE,
  ADMIN_OAUTH_CLIENT_PER_PAGE_OPTIONS,
  adminOAuthClientCustomFilterPatch,
  adminOAuthClientFilterPatch,
  getAppliedAdminOAuthClientFilters,
  getAdminOAuthClientCustomValues,
  getAdminOAuthClientFilterFields,
  getAdminOAuthClientFilterValues,
  getAdminOAuthClientSearchFields,
  getAdminOAuthClientSearchFilters,
  isAdminOAuthClientDateFilterValid,
  normalizeAdminOAuthClientSearch,
  parseAdminOAuthClientSearchFilters,
  updateAdminOAuthClientSearchFilters,
} from "@/lib/admin-oauth-clients";
import {
  DataTableControls,
  DataTableFilterChips,
  DataTableFilterPopover,
  DataTableSearch,
} from "@/components/data-table/data-table-controls";
import {
  clampColumnWidth,
  columnWidthVar,
  columnWidthVars,
  frozenColumnFields,
  stickyColumnLeft,
  sumOfColumnWidths,
} from "@/lib/data-table-columns";
import {
  isDefaultColumnLayout,
  loadColumnPreferences,
  saveColumnPreferences,
} from "@/lib/data-table-preferences";

type ClientAction = {
  readonly client: AdminOAuthClient;
  readonly data: UpdateAdminOAuthClientRequest;
  readonly title: string;
  readonly description: string;
  readonly confirmLabel: string;
};

type BrokerSettingKey = keyof UpdateBrokerSettingsRequest;

type SettingAction = {
  readonly field: BrokerSettingKey;
  readonly value: boolean | null;
  readonly title: string;
  readonly description: string;
  readonly confirmLabel: string;
};

const SETTING_LABELS: Record<BrokerSettingKey, string> = {
  broker_require_sender_constraint: "Require sender constraint",
  broker_require_admin_capability: "Require admin broker capability",
};

const BROKER_BINDING_SCOPE = "urn:nyxid:scope:broker_binding";
const DEFAULT_SORT: AdminOAuthClientSort = "-created_at";

const FALLBACK_SORTS: readonly AdminOAuthClientSort[] = [
  "-created_at",
  "created_at",
  "client_name",
  "-client_name",
  "client_type",
  "-client_type",
  "created_by",
  "-created_by",
  "broker",
  "-broker",
  "allowed_scopes",
  "-allowed_scopes",
  "-is_active",
  "is_active",
];

type OAuthClientSortField =
  | "client_name"
  | "client_type"
  | "created_by"
  | "broker"
  | "is_active"
  | "allowed_scopes"
  | "created_at";

type OAuthClientColumn = {
  readonly field: OAuthClientSortField;
  readonly label: string;
  /** Starting width, and the width a double-click on the resize handle restores. */
  readonly defaultWidth: number;
  readonly cellClassName?: string;
};

const OAUTH_CLIENT_COLUMNS: readonly OAuthClientColumn[] = [
  {
    field: "client_name",
    label: "Client",
    defaultWidth: 260,
  },
  {
    field: "client_type",
    label: "Type",
    defaultWidth: 140,
  },
  {
    field: "created_by",
    label: "Created By",
    defaultWidth: 200,
    cellClassName: "font-mono text-[11px] text-muted-foreground",
  },
  {
    field: "broker",
    label: "Broker",
    defaultWidth: 280,
  },
  {
    field: "is_active",
    label: "Status",
    defaultWidth: 170,
  },
  {
    field: "allowed_scopes",
    label: "Allowed Scopes",
    defaultWidth: 320,
  },
  {
    field: "created_at",
    label: "Created",
    defaultWidth: 190,
    cellClassName: "whitespace-nowrap text-sm text-muted-foreground",
  },
];

const DEFAULT_COLUMN_ORDER = OAUTH_CLIENT_COLUMNS.map((column) => column.field);

const MIN_COLUMN_WIDTH = 96;
const MAX_COLUMN_WIDTH = 640;
const COLUMN_RESIZE_STEP = 16;

type ColumnWidths = Readonly<Record<OAuthClientSortField, number>>;

const DEFAULT_COLUMN_WIDTHS = Object.fromEntries(
  OAUTH_CLIENT_COLUMNS.map((column) => [column.field, column.defaultWidth]),
) as ColumnWidths;

type ColumnPreferences = DataTableColumnPreferences<OAuthClientSortField>;

const DEFAULT_COLUMN_PREFERENCES: ColumnPreferences = {
  order: DEFAULT_COLUMN_ORDER,
  frozenThrough: null,
  widths: DEFAULT_COLUMN_WIDTHS,
};

const COLUMN_WIDTH_BOUNDS = { min: MIN_COLUMN_WIDTH, max: MAX_COLUMN_WIDTH };

/** Bump the version suffix when a change makes stored layouts unreadable. */
const COLUMN_PREFERENCES_KEY = "nyxid.table.admin-oauth-clients.columns.v1";

type ColumnDropTarget = {
  readonly field: OAuthClientSortField;
  readonly position: "before" | "after";
};

/**
 * Divider marking the frozen edge, drawn as a pseudo-element rather than a
 * `border-r`. A collapsed border belongs to the table's border grid, not to the
 * cell, so it stays behind with the grid when a sticky cell is offset -- the
 * frozen edge would lose its line as soon as the table scrolls horizontally.
 * What the sticky cell paints itself travels with it at every scroll offset.
 */
const FROZEN_EDGE_CLASS =
  "shadow-[3px_0_6px_-4px_rgba(0,0,0,0.35)] before:pointer-events-none before:absolute before:inset-y-0 before:right-0 before:z-10 before:w-0.5 before:bg-border before:content-['']";

function getColumn(field: OAuthClientSortField): OAuthClientColumn {
  const column = OAUTH_CLIENT_COLUMNS.find((item) => item.field === field);
  if (!column) throw new Error(`Unknown OAuth client column: ${field}`);
  return column;
}

function isColumnField(value: unknown): value is OAuthClientSortField {
  return OAUTH_CLIENT_COLUMNS.some((column) => column.field === value);
}

function moveColumnToIndex(
  order: readonly OAuthClientSortField[],
  field: OAuthClientSortField,
  targetIndex: number,
): OAuthClientSortField[] {
  const sourceIndex = order.indexOf(field);
  if (sourceIndex < 0) return [...order];
  const next = order.filter((item) => item !== field);
  next.splice(Math.max(0, Math.min(targetIndex, next.length)), 0, field);
  return next;
}

function reorderColumnsForDrop(
  order: readonly OAuthClientSortField[],
  source: OAuthClientSortField,
  target: OAuthClientSortField,
  position: ColumnDropTarget["position"],
): OAuthClientSortField[] {
  if (source === target) return [...order];
  const withoutSource = order.filter((field) => field !== source);
  const targetIndex = withoutSource.indexOf(target);
  if (targetIndex < 0) return [...order];
  const insertionIndex = targetIndex + (position === "after" ? 1 : 0);
  withoutSource.splice(insertionIndex, 0, source);
  return withoutSource;
}

function resizeColumnWidth(width: number): number {
  return clampColumnWidth(width, MIN_COLUMN_WIDTH, MAX_COLUMN_WIDTH);
}

const DEFAULT_COLUMN_SORT: Record<OAuthClientSortField, AdminOAuthClientSort> =
  {
    client_name: "client_name",
    client_type: "client_type",
    created_by: "created_by",
    broker: "-broker",
    is_active: "-is_active",
    allowed_scopes: "allowed_scopes",
    created_at: "-created_at",
  };

type OAuthClientSearchEditTarget =
  | { readonly kind: "all"; readonly value: string }
  | {
      readonly kind: "field";
      readonly field: AdminOAuthClientSearchFieldKey;
      readonly value: string;
    };

function sortDirection(
  sort: AdminOAuthClientSort,
  field: OAuthClientSortField,
): "ascending" | "descending" | undefined {
  if (sort === field) return "ascending";
  if (sort === `-${field}`) return "descending";
  return undefined;
}

function nextColumnSort(
  sort: AdminOAuthClientSort,
  field: OAuthClientSortField,
): AdminOAuthClientSort {
  const ascending = field as AdminOAuthClientSort;
  const descending = `-${field}` as AdminOAuthClientSort;
  if (sort === ascending) return descending;
  if (sort === descending) return ascending;
  return DEFAULT_COLUMN_SORT[field];
}

function ReorderableTableHead({
  column,
  sort,
  disabled,
  frozen,
  lastFrozen,
  stickyLeft,
  width,
  resizing,
  dragging,
  dropPosition,
  onSort,
  onDragStart,
  onDragOver,
  onDrop,
  onDragEnd,
  onMoveByKeyboard,
  onToggleFreeze,
  onResizeStart,
  onResizeByKeyboard,
  onResizeReset,
}: {
  readonly column: OAuthClientColumn;
  readonly sort: AdminOAuthClientSort;
  readonly disabled: boolean;
  readonly frozen: boolean;
  readonly lastFrozen: boolean;
  readonly stickyLeft: string | undefined;
  readonly width: number;
  readonly resizing: boolean;
  readonly dragging: boolean;
  readonly dropPosition: ColumnDropTarget["position"] | undefined;
  readonly onSort: (sort: AdminOAuthClientSort) => void;
  readonly onDragStart: (
    field: OAuthClientSortField,
    event: React.DragEvent<HTMLButtonElement>,
  ) => void;
  readonly onDragOver: (
    field: OAuthClientSortField,
    event: React.DragEvent<HTMLTableCellElement>,
  ) => void;
  readonly onDrop: (
    field: OAuthClientSortField,
    event: React.DragEvent<HTMLTableCellElement>,
  ) => void;
  readonly onDragEnd: () => void;
  readonly onMoveByKeyboard: (
    field: OAuthClientSortField,
    key: "ArrowLeft" | "ArrowRight" | "Home" | "End",
  ) => void;
  readonly onToggleFreeze: (field: OAuthClientSortField) => void;
  readonly onResizeStart: (
    field: OAuthClientSortField,
    event: React.PointerEvent<HTMLDivElement>,
  ) => void;
  readonly onResizeByKeyboard: (
    field: OAuthClientSortField,
    key: "ArrowLeft" | "ArrowRight" | "Home",
  ) => void;
  readonly onResizeReset: (field: OAuthClientSortField) => void;
}) {
  const { field, label } = column;
  const direction = sortDirection(sort, field);
  const Icon =
    direction === "ascending"
      ? ArrowUp
      : direction === "descending"
        ? ArrowDown
        : ArrowUpDown;
  const next = nextColumnSort(sort, field);
  const nextDirection = next.startsWith("-") ? "descending" : "ascending";

  return (
    <TableHead
      aria-label={label}
      aria-sort={direction ?? "none"}
      data-column={field}
      data-dragging={dragging || undefined}
      data-drop-position={dropPosition}
      data-frozen={frozen || undefined}
      data-frozen-edge={lastFrozen || undefined}
      data-resizing={resizing || undefined}
      style={frozen ? { left: stickyLeft } : undefined}
      onDragOver={(event) => onDragOver(field, event)}
      onDrop={(event) => onDrop(field, event)}
      className={cn(
        "group/header relative h-10 p-0 transition-colors",
        frozen && "sticky z-30 bg-card",
        lastFrozen && FROZEN_EDGE_CLASS,
        direction && "text-foreground",
      )}
    >
      {/* Tint overlays, never a background swap: a frozen header must keep an
          opaque background or the columns scrolling underneath show through. */}
      <span
        className="pointer-events-none absolute inset-0 bg-accent opacity-0 transition-opacity group-hover/header:opacity-100"
        aria-hidden="true"
      />
      {direction && (
        <span
          className="pointer-events-none absolute inset-0 bg-primary/[0.07]"
          aria-hidden="true"
        />
      )}
      {dropPosition && (
        <span
          data-testid={`column-drop-indicator-${field}`}
          className={cn(
            "pointer-events-none absolute inset-y-0 z-30 w-0.5 bg-primary",
            dropPosition === "before" ? "left-0" : "right-0",
          )}
          aria-hidden="true"
        />
      )}
      <div
        className={cn(
          // Right padding keeps the pin button clear of the resize handle.
          "flex h-full min-w-0 items-center pr-1.5",
          dragging && "opacity-55",
        )}
      >
        <button
          type="button"
          draggable
          aria-label={`Move ${label} column`}
          title={`Move ${label} column`}
          onDragStart={(event) => onDragStart(field, event)}
          onDragEnd={onDragEnd}
          onKeyDown={(event) => {
            if (
              event.key === "ArrowLeft" ||
              event.key === "ArrowRight" ||
              event.key === "Home" ||
              event.key === "End"
            ) {
              event.preventDefault();
              onMoveByKeyboard(field, event.key);
            }
          }}
          className="flex h-full w-7 shrink-0 cursor-grab items-center justify-center text-muted-foreground opacity-0 outline-none transition-opacity hover:text-foreground focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring active:cursor-grabbing group-hover/header:opacity-100 group-focus-within/header:opacity-100"
        >
          <GripVertical className="h-3.5 w-3.5" aria-hidden="true" />
        </button>
        <button
          type="button"
          className={cn(
            "group/sort flex h-full min-w-0 flex-1 items-center gap-1.5 text-left text-[10px] font-semibold uppercase tracking-normal outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50",
            direction && "text-foreground",
          )}
          disabled={disabled}
          aria-label={`Sort by ${label}, ${nextDirection}`}
          title={`Sort by ${label}, ${nextDirection}`}
          onClick={() => onSort(next)}
        >
          <span className="truncate">{label}</span>
          <Icon
            className={cn(
              "h-3.5 w-3.5 shrink-0",
              direction
                ? "text-primary"
                : "text-muted-foreground/55 group-hover/sort:text-muted-foreground",
            )}
            aria-hidden="true"
          />
        </button>
        <button
          type="button"
          aria-label={
            lastFrozen
              ? `Unfreeze columns through ${label}`
              : `Freeze columns through ${label}`
          }
          title={lastFrozen ? "Unfreeze columns" : "Freeze through this column"}
          aria-pressed={lastFrozen}
          onClick={() => onToggleFreeze(field)}
          className={cn(
            "flex h-full w-7 shrink-0 items-center justify-center outline-none transition-opacity hover:text-foreground focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring group-hover/header:opacity-100 group-focus-within/header:opacity-100",
            lastFrozen
              ? "text-primary opacity-100"
              : "text-muted-foreground opacity-0",
          )}
        >
          {lastFrozen ? (
            <PinOff className="h-3.5 w-3.5" aria-hidden="true" />
          ) : (
            <Pin className="h-3.5 w-3.5" aria-hidden="true" />
          )}
        </button>
      </div>
      <div
        role="separator"
        aria-orientation="vertical"
        aria-label={`Resize ${label} column`}
        aria-valuenow={width}
        aria-valuemin={MIN_COLUMN_WIDTH}
        aria-valuemax={MAX_COLUMN_WIDTH}
        tabIndex={0}
        title={`Drag to resize ${label}, double-click to reset`}
        onPointerDown={(event) => onResizeStart(field, event)}
        onDoubleClick={() => onResizeReset(field)}
        onKeyDown={(event) => {
          if (
            event.key === "ArrowLeft" ||
            event.key === "ArrowRight" ||
            event.key === "Home"
          ) {
            event.preventDefault();
            onResizeByKeyboard(field, event.key);
          }
        }}
        className="group/resize absolute inset-y-0 right-0 z-40 flex w-1.5 cursor-col-resize touch-none justify-center outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
      >
        <span
          aria-hidden="true"
          className={cn(
            "h-full w-0.5 transition-colors group-hover/resize:bg-primary",
            resizing && "bg-primary",
          )}
        />
      </div>
    </TableHead>
  );
}

export function AdminOAuthClientsPage() {
  const navigate = useNavigate();
  const routeSearch = useSearch({
    strict: false,
  }) as AdminOAuthClientSearchState;
  const currentUser = useAuthStore((s) => s.user);
  const canWrite = canAdminWrite(currentUser);
  const [columnPreferences, setColumnPreferences] = useState(() =>
    loadColumnPreferences(
      COLUMN_PREFERENCES_KEY,
      DEFAULT_COLUMN_PREFERENCES,
      COLUMN_WIDTH_BOUNDS,
    ),
  );
  const [draggingColumn, setDraggingColumn] =
    useState<OAuthClientSortField | null>(null);
  const [columnDropTarget, setColumnDropTarget] =
    useState<ColumnDropTarget | null>(null);
  const [columnAnnouncement, setColumnAnnouncement] = useState("");
  const draggingColumnRef = useRef<OAuthClientSortField | null>(null);
  const [resizingColumn, setResizingColumn] =
    useState<OAuthClientSortField | null>(null);
  const detachResizeRef = useRef<(() => void) | null>(null);
  const tableRef = useRef<HTMLTableElement>(null);
  const appliedSearch = routeSearch.search ?? "";
  const [searchDraft, setSearchDraft] = useState("");
  const [selectedSearchField, setSelectedSearchField] =
    useState<AdminOAuthClientSearchFieldKey | null>(null);
  const [editingSearch, setEditingSearch] =
    useState<OAuthClientSearchEditTarget | null>(null);
  const previousSearchRouteRef = useRef(
    `${routeSearch.search ?? ""}\u0000${routeSearch.search_filters ?? ""}`,
  );
  const [pendingClientAction, setPendingClientAction] =
    useState<ClientAction | null>(null);
  const [pendingSettingAction, setPendingSettingAction] =
    useState<SettingAction | null>(null);
  const [filterPopoverOpen, setFilterPopoverOpen] = useState(false);
  const [selectedFilterKey, setSelectedFilterKey] =
    useState<AdminOAuthClientFilterKey>("is_active");
  const searchInputRef = useRef<HTMLInputElement>(null);

  const listParams: AdminOAuthClientListParams = {
    page: routeSearch.page ?? 1,
    per_page: routeSearch.per_page ?? ADMIN_OAUTH_CLIENT_DEFAULT_PER_PAGE,
    search: routeSearch.search,
    search_filters: routeSearch.search_filters,
    custom_filters: routeSearch.custom_filters,
    client_type: routeSearch.client_type,
    creator_type: routeSearch.creator_type,
    broker: routeSearch.broker,
    is_active: routeSearch.is_active,
    scope: routeSearch.scope,
    created_dates: routeSearch.created_dates,
    created_from: routeSearch.created_from,
    created_to: routeSearch.created_to,
    sort: routeSearch.sort ?? DEFAULT_SORT,
  };

  const { data, isLoading, isFetching, isPlaceholderData, error, refetch } =
    useAdminOAuthClients(listParams);
  const {
    data: brokerSettings,
    isLoading: settingsLoading,
    error: settingsError,
  } = useBrokerSettings(canWrite);
  const updateClient = useUpdateAdminOAuthClient();
  const updateSettings = useUpdateBrokerSettings();

  const clients = data?.clients ?? [];
  const total = data?.total ?? 0;
  const totalPages = Math.max(1, Math.ceil(total / listParams.per_page));
  const displayedPage = data?.page ?? listParams.page;
  const displayedPerPage = data?.per_page ?? listParams.per_page;
  const displayedTotalPages = Math.max(1, Math.ceil(total / displayedPerPage));
  const filterOptions = data?.filter_options;
  const filterFields = getAdminOAuthClientFilterFields(filterOptions);
  const searchFields = getAdminOAuthClientSearchFields(filterOptions);
  const appliedSearchFilters = getAdminOAuthClientSearchFilters(listParams);
  const appliedStructuredFilters = getAppliedAdminOAuthClientFilters(
    filterFields,
    listParams,
  );
  const filterSelections: AdminOAuthClientFilterSelections = {};
  const customFilterSelections: AdminOAuthClientFilterSelections = {};
  for (const field of filterFields) {
    filterSelections[field.key] = getAdminOAuthClientFilterValues(
      listParams,
      field.key,
    );
    customFilterSelections[field.key] = getAdminOAuthClientCustomValues(
      listParams,
      field.key,
    );
  }
  const availableSorts = new Set(filterOptions?.sorts ?? FALLBACK_SORTS);
  const hasActiveFilters = Boolean(
    listParams.search ||
    appliedSearchFilters.length > 0 ||
    appliedStructuredFilters.length > 0,
  );
  const columnOrder = columnPreferences.order;
  const frozenThrough = columnPreferences.frozenThrough;
  const columnWidths = columnPreferences.widths;
  const frozenFields = frozenColumnFields(columnOrder, frozenThrough);

  const updateListSearch = useCallback(
    (patch: Partial<AdminOAuthClientSearchState>, replace = false) => {
      void navigate({
        to: "/admin/oauth-clients",
        search: (previous) =>
          normalizeAdminOAuthClientSearch({ ...previous, ...patch }),
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
      client_type: undefined,
      creator_type: undefined,
      broker: undefined,
      is_active: undefined,
      scope: undefined,
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
    field: AdminOAuthClientSearchFieldKey,
    value: string,
    navigate = true,
  ): string | undefined {
    const current = parseAdminOAuthClientSearchFilters(
      listParams.search_filters,
    );
    const values =
      current
        ?.find((group) => group.field === field)
        ?.values.filter(
          (item) => item.toLocaleLowerCase() !== value.toLocaleLowerCase(),
        ) ?? [];
    const next = updateAdminOAuthClientSearchFilters(
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
      const current = parseAdminOAuthClientSearchFilters(nextScopedSearch);
      const remaining =
        current
          ?.find((group) => group.field === editingSearch.field)
          ?.values.filter(
            (value) =>
              value.toLocaleLowerCase() !==
              editingSearch.value.toLocaleLowerCase(),
          ) ?? [];
      nextScopedSearch = updateAdminOAuthClientSearchFilters(
        nextScopedSearch,
        editingSearch.field,
        remaining.length > 0 ? remaining : undefined,
      );
    }

    if (selectedSearchField === null) {
      nextLegacySearch = term;
    } else {
      const current =
        parseAdminOAuthClientSearchFilters(nextScopedSearch) ?? [];
      const values =
        current.find((group) => group.field === selectedSearchField)?.values ??
        [];
      const duplicate = values.some(
        (value) => value.toLocaleLowerCase() === term.toLocaleLowerCase(),
      );
      const nextValues = duplicate ? values : [...values, term];
      const encoded = updateAdminOAuthClientSearchFilters(
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
    field: AdminOAuthClientSearchFieldKey,
    value: string,
  ) {
    setSelectedSearchField(field);
    setSearchDraft(value);
    setEditingSearch({ kind: "field", field, value });
    focusSearchInput();
  }

  function applyStructuredFilters(
    selections: AdminOAuthClientFilterSelections,
    customSelections: AdminOAuthClientFilterSelections,
  ) {
    const patch: Partial<AdminOAuthClientSearchState> = { page: undefined };
    for (const field of filterFields) {
      const fieldPatch = adminOAuthClientFilterPatch(
        field.key,
        selections[field.key],
      );
      if (!fieldPatch) return;
      Object.assign(patch, fieldPatch);
    }
    Object.assign(patch, adminOAuthClientCustomFilterPatch(customSelections));
    updateListSearch(patch);
  }

  function removeStructuredFilter(
    key: AdminOAuthClientFilterKey,
    custom: boolean,
  ) {
    if (custom) {
      const remaining = { ...customFilterSelections, [key]: undefined };
      updateListSearch({
        page: undefined,
        ...adminOAuthClientCustomFilterPatch(remaining),
      });
      return;
    }
    const patch = adminOAuthClientFilterPatch(key, undefined);
    if (patch) updateListSearch({ page: undefined, ...patch });
  }

  function editStructuredFilter(key: AdminOAuthClientFilterKey) {
    setSelectedFilterKey(key);
    setFilterPopoverOpen(true);
  }

  function updateSort(sort: AdminOAuthClientSort) {
    updateListSearch({
      page: undefined,
      sort: sort === DEFAULT_SORT ? undefined : sort,
    });
  }

  function canSortColumn(field: OAuthClientSortField) {
    return (
      availableSorts.has(field as AdminOAuthClientSort) &&
      availableSorts.has(`-${field}` as AdminOAuthClientSort)
    );
  }

  function announceColumnPosition(
    field: OAuthClientSortField,
    order: readonly OAuthClientSortField[],
  ) {
    setColumnAnnouncement(
      `${getColumn(field).label} column moved to position ${String(
        order.indexOf(field) + 1,
      )} of ${String(order.length)}`,
    );
  }

  function updateColumnOrder(
    nextOrder: readonly OAuthClientSortField[],
    movedField: OAuthClientSortField,
  ) {
    setColumnPreferences((previous) => ({
      ...previous,
      order: nextOrder,
    }));
    announceColumnPosition(movedField, nextOrder);
  }

  function handleColumnDragStart(
    field: OAuthClientSortField,
    event: React.DragEvent<HTMLButtonElement>,
  ) {
    draggingColumnRef.current = field;
    setDraggingColumn(field);
    setColumnDropTarget(null);
    event.dataTransfer.effectAllowed = "move";
    event.dataTransfer.setData("text/plain", field);
  }

  function dropPositionForEvent(
    event: React.DragEvent<HTMLTableCellElement>,
  ): ColumnDropTarget["position"] {
    const bounds = event.currentTarget.getBoundingClientRect();
    return event.clientX < bounds.left + bounds.width / 2 ? "before" : "after";
  }

  function handleColumnDragOver(
    field: OAuthClientSortField,
    event: React.DragEvent<HTMLTableCellElement>,
  ) {
    const source = draggingColumnRef.current;
    if (!source || source === field) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "move";
    const nextTarget: ColumnDropTarget = {
      field,
      position: dropPositionForEvent(event),
    };
    setColumnDropTarget((current) =>
      current?.field === nextTarget.field &&
      current.position === nextTarget.position
        ? current
        : nextTarget,
    );
  }

  function clearColumnDragState() {
    draggingColumnRef.current = null;
    setDraggingColumn(null);
    setColumnDropTarget(null);
  }

  function handleColumnDrop(
    field: OAuthClientSortField,
    event: React.DragEvent<HTMLTableCellElement>,
  ) {
    const transferredField = event.dataTransfer.getData("text/plain");
    const source =
      draggingColumnRef.current ??
      (isColumnField(transferredField) ? transferredField : null);
    if (!source || source === field) {
      clearColumnDragState();
      return;
    }

    event.preventDefault();
    const position =
      columnDropTarget?.field === field
        ? columnDropTarget.position
        : dropPositionForEvent(event);
    const nextOrder = reorderColumnsForDrop(
      columnOrder,
      source,
      field,
      position,
    );
    updateColumnOrder(nextOrder, source);
    clearColumnDragState();
  }

  function handleColumnKeyboardMove(
    field: OAuthClientSortField,
    key: "ArrowLeft" | "ArrowRight" | "Home" | "End",
  ) {
    const sourceIndex = columnOrder.indexOf(field);
    const targetIndex =
      key === "Home"
        ? 0
        : key === "End"
          ? columnOrder.length - 1
          : key === "ArrowLeft"
            ? Math.max(0, sourceIndex - 1)
            : Math.min(columnOrder.length - 1, sourceIndex + 1);
    if (sourceIndex < 0 || sourceIndex === targetIndex) return;
    updateColumnOrder(
      moveColumnToIndex(columnOrder, field, targetIndex),
      field,
    );
  }

  function toggleFrozenColumns(field: OAuthClientSortField) {
    setColumnPreferences((previous) => ({
      ...previous,
      frozenThrough: previous.frozenThrough === field ? null : field,
    }));
  }

  function commitColumnWidth(field: OAuthClientSortField, width: number) {
    setColumnPreferences((previous) => ({
      ...previous,
      widths: { ...previous.widths, [field]: width },
    }));
    setColumnAnnouncement(
      `${getColumn(field).label} column resized to ${String(width)} pixels`,
    );
  }

  /**
   * The drag is tracked on the window, so it survives the pointer leaving the
   * handle, and it writes the column's width variable straight to the table
   * rather than through state -- the browser then reflows the columns, the
   * table width, and any frozen offsets without re-rendering a single row.
   *
   * The listeners go on at pointerdown rather than in an effect keyed on the
   * resizing column: an effect does not run until after the next commit, and a
   * pointer event landing in that gap would be dropped -- a `pointerup` there
   * would strand the table mid-resize.
   */
  function handleResizeStart(
    field: OAuthClientSortField,
    event: React.PointerEvent<HTMLDivElement>,
  ) {
    if (event.button !== 0) return;
    // Keep the pointer press off the header's sort button and drag handle.
    event.preventDefault();
    event.stopPropagation();

    const startWidth = columnWidths[field];
    const resize = { startX: event.clientX, width: startWidth };
    setResizingColumn(field);

    function handleMove(moveEvent: PointerEvent) {
      resize.width = resizeColumnWidth(
        startWidth + moveEvent.clientX - resize.startX,
      );
      tableRef.current?.style.setProperty(
        columnWidthVar(field),
        `${String(resize.width)}px`,
      );
    }

    function handleEnd() {
      detachResize();
      setResizingColumn(null);
      if (resize.width !== startWidth) commitColumnWidth(field, resize.width);
    }

    function detachResize() {
      detachResizeRef.current = null;
      window.removeEventListener("pointermove", handleMove);
      window.removeEventListener("pointerup", handleEnd);
      window.removeEventListener("pointercancel", handleEnd);
    }

    detachResizeRef.current = detachResize;
    window.addEventListener("pointermove", handleMove);
    window.addEventListener("pointerup", handleEnd);
    window.addEventListener("pointercancel", handleEnd);
  }

  // Drop a drag still in flight if the table unmounts under it.
  useEffect(() => () => detachResizeRef.current?.(), []);

  useEffect(() => {
    saveColumnPreferences(
      COLUMN_PREFERENCES_KEY,
      columnPreferences,
      DEFAULT_COLUMN_PREFERENCES,
    );
  }, [columnPreferences]);

  function resetColumnLayout() {
    setColumnPreferences(DEFAULT_COLUMN_PREFERENCES);
    setColumnAnnouncement("Column layout reset");
  }

  function handleResizeKeyboard(
    field: OAuthClientSortField,
    key: "ArrowLeft" | "ArrowRight" | "Home",
  ) {
    const current = columnWidths[field];
    const next =
      key === "Home"
        ? getColumn(field).defaultWidth
        : resizeColumnWidth(
            current +
              (key === "ArrowRight" ? COLUMN_RESIZE_STEP : -COLUMN_RESIZE_STEP),
          );
    if (next === current) return;
    commitColumnWidth(field, next);
  }

  function resetColumnWidth(field: OAuthClientSortField) {
    const { defaultWidth } = getColumn(field);
    if (columnWidths[field] === defaultWidth) return;
    commitColumnWidth(field, defaultWidth);
  }

  function stageBrokerCapability(client: AdminOAuthClient, enabled: boolean) {
    const removingLegacyScope =
      !enabled &&
      client.allowed_scopes.split(/\s+/).includes(BROKER_BINDING_SCOPE);
    const data: UpdateAdminOAuthClientRequest = enabled
      ? { broker_capability_enabled: true }
      : {
          broker_capability_enabled: false,
          allowed_scopes: removeBrokerBindingScope(client.allowed_scopes),
        };

    setPendingClientAction({
      client,
      data,
      title: enabled ? "Enable broker capability" : "Disable broker capability",
      description: enabled
        ? "This grants this app durable act-as-user broker capability. Only enable it for a reviewed OAuth client."
        : removingLegacyScope
          ? "This clears the admin broker flag and removes the legacy broker-binding scope so this app no longer remains broker-capable through scope-triggered compatibility."
          : "This removes durable act-as-user broker capability from this OAuth client.",
      confirmLabel: enabled ? "Enable capability" : "Disable capability",
    });
  }

  function stageActive(client: AdminOAuthClient, isActive: boolean) {
    setPendingClientAction({
      client,
      data: { is_active: isActive },
      title: isActive ? "Activate OAuth client" : "Deactivate OAuth client",
      description: isActive
        ? "This allows the OAuth client to participate in new authorization and token flows."
        : "This deactivates the OAuth client and clears its existing consents, refresh tokens, and pending authorization codes.",
      confirmLabel: isActive ? "Activate client" : "Deactivate client",
    });
  }

  async function confirmClientAction() {
    if (!pendingClientAction) return;
    try {
      await updateClient.mutateAsync({
        clientId: pendingClientAction.client.id,
        data: pendingClientAction.data,
      });
      toast.success("OAuth client updated");
      setPendingClientAction(null);
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to update OAuth client",
      );
    }
  }

  function stageSetting(field: BrokerSettingKey, value: boolean | null) {
    const label = SETTING_LABELS[field];
    const isReset = value === null;
    const enabled = value === true;
    const description =
      field === "broker_require_sender_constraint"
        ? settingSenderConstraintCopy(value)
        : settingAdminCapabilityCopy(value);

    setPendingSettingAction({
      field,
      value,
      title: isReset
        ? `Reset ${label}`
        : `${enabled ? "Enable" : "Disable"} ${label}`,
      description,
      confirmLabel: isReset
        ? "Reset to env default"
        : enabled
          ? "Enable setting"
          : "Disable setting",
    });
  }

  async function confirmSettingAction() {
    if (!pendingSettingAction) return;
    try {
      await updateSettings.mutateAsync({
        [pendingSettingAction.field]: pendingSettingAction.value,
      });
      toast.success("Broker settings updated");
      setPendingSettingAction(null);
    } catch (err) {
      toast.error(
        err instanceof ApiError
          ? err.message
          : "Failed to update broker settings",
      );
    }
  }

  function renderOAuthClientCell(
    client: AdminOAuthClient,
    field: OAuthClientSortField,
  ): React.ReactNode {
    switch (field) {
      case "client_name":
        return (
          <div className="min-w-0 space-y-1">
            <p className="truncate text-sm font-medium text-foreground">
              {client.client_name}
            </p>
            <p className="truncate font-mono text-[11px] text-muted-foreground">
              {client.id}
            </p>
          </div>
        );
      case "client_type":
        return <Badge variant="secondary">{client.client_type}</Badge>;
      case "created_by":
        return (
          <span className="break-all">{client.created_by ?? "ownerless"}</span>
        );
      case "broker":
        return canWrite ? (
          <div className="flex items-center gap-2.5">
            <Switch
              aria-label={`Toggle broker capability for ${client.client_name} (${client.id})`}
              checked={client.broker_capability_effective}
              disabled={
                isFetching ||
                (updateClient.isPending &&
                  updateClient.variables?.clientId === client.id)
              }
              onCheckedChange={(checked) =>
                stageBrokerCapability(client, checked)
              }
            />
            <span className="whitespace-nowrap text-[11px] font-medium text-foreground">
              {client.broker_capability_effective
                ? "Broker enabled"
                : "No broker"}
            </span>
            <BrokerSourceBadge source={client.broker_capability_source} />
          </div>
        ) : (
          <div className="flex items-center gap-2">
            <ClientStateBadge
              active={client.broker_capability_effective}
              activeLabel="Broker enabled"
              inactiveLabel="No broker"
            />
            <BrokerSourceBadge source={client.broker_capability_source} />
          </div>
        );
      case "is_active":
        return canWrite ? (
          <div className="flex items-center gap-2.5">
            <Switch
              aria-label={`Toggle active status for ${client.client_name} (${client.id})`}
              checked={client.is_active}
              disabled={
                isFetching ||
                (updateClient.isPending &&
                  updateClient.variables?.clientId === client.id)
              }
              onCheckedChange={(checked) => stageActive(client, checked)}
            />
            <span className="whitespace-nowrap text-[11px] font-medium text-foreground">
              {client.is_active ? "Active" : "Inactive"}
            </span>
          </div>
        ) : (
          <ClientStateBadge
            active={client.is_active}
            activeLabel="Active"
            inactiveLabel="Inactive"
          />
        );
      case "allowed_scopes":
        return <ScopeList scopes={client.allowed_scopes} />;
      case "created_at":
        return formatDate(client.created_at);
    }
  }

  return (
    <div className="space-y-8">
      <PageHeader
        title="OAuth Clients"
        description="Manage platform OAuth clients, including dynamic registrations used by broker-capable apps."
      />

      {canWrite && (
        <BrokerSettingsSection
          settings={brokerSettings}
          isLoading={settingsLoading}
          error={settingsError}
          onStage={stageSetting}
          disabled={updateSettings.isPending}
        />
      )}

      <section
        className="m-0 overflow-hidden rounded-lg border border-border/60 bg-card"
        aria-label="OAuth clients"
      >
        <DataTableControls
          search={
            <DataTableSearch
              fields={searchFields}
              value={searchDraft}
              selectedField={selectedSearchField}
              inputRef={searchInputRef}
              ariaLabel="Search OAuth clients"
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
                isAdminOAuthClientDateFilterValid(values)
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

        {isLoading || (isPlaceholderData && clients.length === 0) ? (
          <div
            className="space-y-2 p-3"
            role="status"
            aria-live="polite"
            aria-label={
              isLoading ? "Loading OAuth clients" : "Updating OAuth clients"
            }
          >
            {Array.from({ length: 5 }).map((_, i) => (
              <Skeleton
                key={`oauth-client-skel-${String(i)}`}
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
              Failed to load OAuth clients
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
        ) : clients.length === 0 ? (
          <div className="flex min-h-56 flex-col items-center justify-center gap-3 px-4 py-14 text-center">
            <KeyRound className="h-10 w-10 text-muted-foreground" />
            <div>
              <p className="text-sm font-medium text-foreground">
                {hasActiveFilters
                  ? "No clients match these filters"
                  : "No OAuth clients"}
              </p>
              <p className="text-xs text-muted-foreground">
                {hasActiveFilters
                  ? "Adjust or clear the current filters."
                  : "Registered clients will appear here."}
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
                "relative transition-opacity motion-reduce:transition-none",
                isPlaceholderData && "pointer-events-none opacity-60",
                // Hold the resize cursor for the whole drag, wherever it lands.
                resizingColumn && "cursor-col-resize select-none",
              )}
              aria-busy={isFetching}
            >
              <p className="sr-only" aria-live="polite" aria-atomic="true">
                {columnAnnouncement}
              </p>
              {isPlaceholderData && (
                <div
                  data-testid="oauth-clients-refetch-overlay"
                  className="pointer-events-none absolute inset-0 z-30 flex items-start justify-center pt-16 motion-reduce:pt-8"
                  aria-hidden="true"
                >
                  <div className="rounded-full border border-border bg-background/90 p-2.5 shadow-md backdrop-blur-sm">
                    <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
                  </div>
                </div>
              )}
              <Table
                ref={tableRef}
                containerClassName="overscroll-x-none"
                className="table-fixed [&_td]:overflow-hidden [&_td]:py-1.5"
                style={{
                  minWidth: sumOfColumnWidths(columnOrder),
                  ...columnWidthVars(columnOrder, columnWidths),
                }}
              >
                <colgroup>
                  {columnOrder.map((field) => (
                    <col
                      key={field}
                      style={{ width: `var(${columnWidthVar(field)})` }}
                    />
                  ))}
                </colgroup>
                <TableHeader>
                  <TableRow>
                    {columnOrder.map((field) => {
                      const column = getColumn(field);
                      return (
                        <ReorderableTableHead
                          key={field}
                          column={column}
                          sort={listParams.sort}
                          disabled={!canSortColumn(field)}
                          frozen={frozenFields.includes(field)}
                          lastFrozen={frozenThrough === field}
                          stickyLeft={stickyColumnLeft(frozenFields, field)}
                          width={columnWidths[field]}
                          resizing={resizingColumn === field}
                          dragging={draggingColumn === field}
                          dropPosition={
                            columnDropTarget?.field === field
                              ? columnDropTarget.position
                              : undefined
                          }
                          onSort={updateSort}
                          onDragStart={handleColumnDragStart}
                          onDragOver={handleColumnDragOver}
                          onDrop={handleColumnDrop}
                          onDragEnd={clearColumnDragState}
                          onMoveByKeyboard={handleColumnKeyboardMove}
                          onToggleFreeze={toggleFrozenColumns}
                          onResizeStart={handleResizeStart}
                          onResizeByKeyboard={handleResizeKeyboard}
                          onResizeReset={resetColumnWidth}
                        />
                      );
                    })}
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {clients.map((client) => (
                    <TableRow
                      key={client.id}
                      className="group/row hover:bg-muted/45"
                    >
                      {columnOrder.map((field) => {
                        const column = getColumn(field);
                        const frozen = frozenFields.includes(field);
                        return (
                          <TableCell
                            key={field}
                            data-column={field}
                            data-frozen={frozen || undefined}
                            data-frozen-edge={
                              frozenThrough === field || undefined
                            }
                            style={
                              frozen
                                ? {
                                    left: stickyColumnLeft(frozenFields, field),
                                  }
                                : undefined
                            }
                            className={cn(
                              column.cellClassName,
                              // Opaque base so scrolled columns stay hidden; the row
                              // hover tint rides on top as an overlay so a frozen cell
                              // reads the same as the rest of its row.
                              frozen &&
                                "sticky z-20 bg-card after:pointer-events-none after:absolute after:inset-0 after:transition-colors after:duration-300 after:content-[''] group-hover/row:after:bg-muted/45",
                              frozenThrough === field && FROZEN_EDGE_CLASS,
                            )}
                          >
                            {renderOAuthClientCell(client, field)}
                          </TableCell>
                        );
                      })}
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>

            <div className="flex flex-col gap-3 border-t border-border/60 px-3 py-3 sm:flex-row sm:items-center sm:justify-between">
              <p className="text-[11px] text-text-tertiary">
                Showing {String((displayedPage - 1) * displayedPerPage + 1)}-
                {String(Math.min(displayedPage * displayedPerPage, total))} of{" "}
                {String(total)} clients
              </p>
              <div className="flex flex-wrap items-center gap-2">
                {!isDefaultColumnLayout(
                  columnPreferences,
                  DEFAULT_COLUMN_PREFERENCES,
                ) && (
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    onClick={resetColumnLayout}
                  >
                    <RotateCcw
                      className="mr-2 h-3.5 w-3.5"
                      aria-hidden="true"
                    />
                    Reset columns
                  </Button>
                )}
                <Select
                  value={String(listParams.per_page)}
                  disabled={isFetching}
                  onValueChange={(value) => {
                    const rows = Number(value) as AdminOAuthClientPerPage;
                    updateListSearch({
                      page: undefined,
                      per_page:
                        rows === ADMIN_OAUTH_CLIENT_DEFAULT_PER_PAGE
                          ? undefined
                          : rows,
                    });
                  }}
                >
                  <SelectTrigger
                    aria-label="Rows per page"
                    className="w-[112px]"
                  >
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {ADMIN_OAUTH_CLIENT_PER_PAGE_OPTIONS.map((rows) => (
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
                  onClick={() =>
                    updateListSearch({ page: listParams.page + 1 })
                  }
                  aria-label="Next page"
                >
                  <ChevronRight />
                </Button>
              </div>
            </div>
          </>
        )}
      </section>

      <Dialog
        open={canWrite && pendingClientAction !== null}
        onOpenChange={(open) => {
          if (!open) setPendingClientAction(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{pendingClientAction?.title}</DialogTitle>
            <DialogDescription>
              {pendingClientAction?.description}
            </DialogDescription>
          </DialogHeader>
          {pendingClientAction && (
            <div className="rounded-lg bg-muted/50 p-3">
              <p className="text-sm font-medium">
                {pendingClientAction.client.client_name}
              </p>
              <p className="mt-1 font-mono text-[11px] text-muted-foreground">
                {pendingClientAction.client.id}
              </p>
            </div>
          )}
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => setPendingClientAction(null)}
            >
              Cancel
            </Button>
            <Button
              type="button"
              onClick={() => void confirmClientAction()}
              disabled={updateClient.isPending}
            >
              {pendingClientAction?.confirmLabel ?? "Confirm"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog
        open={canWrite && pendingSettingAction !== null}
        onOpenChange={(open) => {
          if (!open) setPendingSettingAction(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{pendingSettingAction?.title}</DialogTitle>
            <DialogDescription>
              {pendingSettingAction?.description}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => setPendingSettingAction(null)}
            >
              Cancel
            </Button>
            <Button
              type="button"
              onClick={() => void confirmSettingAction()}
              disabled={updateSettings.isPending}
            >
              {pendingSettingAction?.confirmLabel ?? "Confirm"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

function BrokerSettingsSection({
  settings,
  isLoading,
  error,
  onStage,
  disabled,
}: {
  readonly settings: BrokerSettingsResponse | undefined;
  readonly isLoading: boolean;
  readonly error: unknown;
  readonly onStage: (field: BrokerSettingKey, value: boolean | null) => void;
  readonly disabled: boolean;
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>Broker Rollout Policy</CardTitle>
        <CardDescription>
          Runtime overrides are applied immediately and fall back to env
          defaults when reset.
        </CardDescription>
      </CardHeader>
      <CardContent>
        {isLoading ? (
          <div className="space-y-2">
            <Skeleton className="h-14 w-full" />
            <Skeleton className="h-14 w-full" />
          </div>
        ) : error ? (
          <div className="flex items-center gap-2 rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive">
            <AlertTriangle className="h-4 w-4" />
            Failed to load broker settings
          </div>
        ) : settings ? (
          <div className="divide-y divide-border/60">
            <BrokerPolicyRow
              label="Require sender constraint"
              setting={settings.broker_require_sender_constraint}
              disabled={disabled}
              onToggle={(checked) =>
                onStage("broker_require_sender_constraint", checked)
              }
              onReset={() => onStage("broker_require_sender_constraint", null)}
            />
            <BrokerPolicyRow
              label="Require admin broker capability"
              setting={settings.broker_require_admin_capability}
              disabled={disabled}
              onToggle={(checked) =>
                onStage("broker_require_admin_capability", checked)
              }
              onReset={() => onStage("broker_require_admin_capability", null)}
            />
          </div>
        ) : null}
      </CardContent>
    </Card>
  );
}

function BrokerPolicyRow({
  label,
  setting,
  disabled,
  onToggle,
  onReset,
}: {
  readonly label: string;
  readonly setting: BrokerPolicyField;
  readonly disabled: boolean;
  readonly onToggle: (checked: boolean) => void;
  readonly onReset: () => void;
}) {
  const overridden = setting.source === "override";
  return (
    <div className="flex flex-col gap-3 py-4 first:pt-0 last:pb-0 sm:flex-row sm:items-center sm:justify-between">
      <div className="min-w-0 space-y-1">
        <div className="flex flex-wrap items-center gap-2">
          <p className="text-sm font-medium text-foreground">{label}</p>
          <Badge variant={overridden ? "default" : "secondary"}>
            {overridden ? "Overridden" : "Env default"}
          </Badge>
          <BoolBadge value={setting.effective} />
        </div>
        <p className="text-xs text-muted-foreground">
          Env default: {setting.env_default ? "enabled" : "disabled"}
          {overridden
            ? ` · Override: ${setting.override ? "enabled" : "disabled"}`
            : ""}
        </p>
      </div>
      <div className="flex items-center gap-2">
        {overridden && (
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={onReset}
            disabled={disabled}
          >
            <RotateCcw className="mr-2 h-3.5 w-3.5" />
            Reset
          </Button>
        )}
        <Switch
          aria-label={`Toggle ${label}`}
          checked={setting.effective}
          disabled={disabled}
          onCheckedChange={onToggle}
        />
      </div>
    </div>
  );
}

function BrokerSourceBadge({
  source,
}: {
  readonly source: AdminOAuthClient["broker_capability_source"];
}) {
  if (source === "none") return null;
  return (
    <Badge variant="secondary" className="whitespace-nowrap">
      {source === "flag" ? "Admin grant" : "Broker scope"}
    </Badge>
  );
}

function ClientStateBadge({
  active,
  activeLabel,
  inactiveLabel,
}: {
  readonly active: boolean;
  readonly activeLabel: string;
  readonly inactiveLabel: string;
}) {
  return (
    <Badge variant={active ? "success" : "secondary"}>
      {active ? activeLabel : inactiveLabel}
    </Badge>
  );
}

function BoolBadge({ value }: { readonly value: boolean }) {
  return (
    <Badge variant={value ? "default" : "secondary"}>
      {value ? "Enabled" : "Disabled"}
    </Badge>
  );
}

function ScopeList({ scopes }: { readonly scopes: string }) {
  const items = scopes.split(/\s+/).filter(Boolean);
  if (items.length === 0) {
    return <span className="text-sm text-muted-foreground">None</span>;
  }
  const visibleItems = items.slice(0, 3);
  const hiddenItems = items.slice(3);

  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          type="button"
          className="flex max-w-full items-center gap-1.5 rounded-md text-left outline-none transition-colors hover:bg-muted/50 focus-visible:ring-2 focus-visible:ring-ring"
          aria-label={`View all ${String(items.length)} allowed scopes: ${items.join(", ")}`}
          title={items.join("\n")}
        >
          {visibleItems.map((scope) => (
            <span
              key={scope}
              className="inline-flex max-w-[112px] truncate rounded-md bg-muted px-2 py-0.5 font-mono text-[10px] font-medium text-muted-foreground"
            >
              {scope}
            </span>
          ))}
          {hiddenItems.length > 0 && (
            <span className="inline-flex h-6 shrink-0 items-center rounded-md border border-border/80 bg-muted px-2 text-[10px] font-medium text-muted-foreground">
              +{String(hiddenItems.length)} more
            </span>
          )}
        </button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-80 rounded-lg p-3">
        <p className="mb-2 text-xs font-semibold text-foreground">
          Allowed scopes
        </p>
        <div className="flex max-h-56 flex-wrap gap-1.5 overflow-y-auto">
          {items.map((scope) => (
            <Badge
              key={scope}
              variant="secondary"
              className="max-w-full break-all font-mono text-[10px]"
            >
              {scope}
            </Badge>
          ))}
        </div>
      </PopoverContent>
    </Popover>
  );
}

function removeBrokerBindingScope(scopes: string): string[] {
  const remaining = scopes
    .split(/\s+/)
    .filter((scope) => scope && scope !== BROKER_BINDING_SCOPE);
  return remaining.length > 0 ? remaining : ["openid"];
}

function settingSenderConstraintCopy(value: boolean | null): string {
  if (value === null) {
    return "This clears the runtime override and returns sender-constraint enforcement to the env default.";
  }
  if (value) {
    return "This requires broker bindings to be DPoP or mTLS sender-constrained. Unpinned broker bindings will be rejected.";
  }
  return "This disables the runtime sender-constraint requirement and allows the env default to be overridden off.";
}

function settingAdminCapabilityCopy(value: boolean | null): string {
  if (value === null) {
    return "This clears the runtime override and returns admin-capability enforcement to the env default.";
  }
  if (value) {
    return "This requires platform-admin provisioning before an OAuth client can receive durable act-as-user broker capability.";
  }
  return "This disables the runtime admin-capability requirement and allows self-service broker-scope clients under this override.";
}
