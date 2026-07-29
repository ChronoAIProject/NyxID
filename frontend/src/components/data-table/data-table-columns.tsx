import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { cn } from "@/lib/utils";
import { TableHead } from "@/components/ui/table";
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
  type DataTableColumnPreferences,
} from "@/lib/data-table-preferences";
import {
  ArrowDown,
  ArrowUp,
  ArrowUpDown,
  GripVertical,
  Pin,
  PinOff,
} from "lucide-react";

/**
 * Reorderable, resizable, freezable columns for the admin data tables.
 *
 * Sort strings follow the server's convention: `field` ascending, `-field`
 * descending. Layout (order, widths, freeze point) is per-table user state and
 * persists to localStorage; sorting is not -- that lives in the URL.
 */
export interface DataTableColumn<Field extends string> {
  readonly field: Field;
  readonly label: string;
  /** Starting width, and the width a double-click on the resize handle restores. */
  readonly defaultWidth: number;
  readonly cellClassName?: string;
}

export type DataTableColumnDropPosition = "before" | "after";

interface ColumnDropTarget<Field extends string> {
  readonly field: Field;
  readonly position: DataTableColumnDropPosition;
}

type ColumnMoveKey = "ArrowLeft" | "ArrowRight" | "Home" | "End";
type ColumnResizeKey = "ArrowLeft" | "ArrowRight" | "Home";

export const MIN_COLUMN_WIDTH = 96;
export const MAX_COLUMN_WIDTH = 640;
const COLUMN_RESIZE_STEP = 16;
const COLUMN_WIDTH_BOUNDS = { min: MIN_COLUMN_WIDTH, max: MAX_COLUMN_WIDTH };

/**
 * Shared table class. Cells carry no vertical padding of their own -- that
 * belongs to [`DataTableCellShell`], which owns the row's minimum height.
 */
export const DATA_TABLE_CLASS_NAME =
  "table-fixed [&_td]:overflow-hidden [&_td]:py-0";

/**
 * Every body cell's vertical rhythm, so each admin table measures the same.
 *
 * A 52px floor that *includes* the padding (border-box), so short rows get real
 * breathing room rather than content squeezed against the edges, and a cell that
 * wraps to a second line grows past the floor keeping the same padding.
 *
 * `[&>*]:min-w-0` lets the child shrink below its intrinsic width, which a flex
 * item otherwise refuses to do -- without it, clamped text overflows its column.
 */
export function DataTableCellShell({
  children,
}: {
  readonly children: React.ReactNode;
}) {
  return (
    <div className="flex min-h-[52px] w-full min-w-0 items-center py-2 [&>*]:min-w-0">
      {children}
    </div>
  );
}

/** Keeps badges inside their column, wrapping rather than overflowing it. */
export function DataTableBadgeCell({
  children,
}: {
  readonly children: React.ReactNode;
}) {
  return (
    <div className="flex min-w-0 flex-wrap items-center gap-1.5">{children}</div>
  );
}

/**
 * Divider marking the frozen edge, drawn as a pseudo-element rather than a
 * `border-r`. A collapsed border belongs to the table's border grid, not to the
 * cell, so it stays behind with the grid when a sticky cell is offset -- the
 * frozen edge would lose its line as soon as the table scrolls horizontally.
 * What the sticky cell paints itself travels with it at every scroll offset.
 */
const FROZEN_EDGE_CLASS =
  "shadow-[3px_0_6px_-4px_rgba(0,0,0,0.35)] before:pointer-events-none before:absolute before:inset-y-0 before:right-0 before:z-10 before:w-0.5 before:bg-border before:content-['']";

export function dataTableSortDirection<Field extends string>(
  sort: string,
  field: Field,
): "ascending" | "descending" | undefined {
  if (sort === field) return "ascending";
  if (sort === `-${field}`) return "descending";
  return undefined;
}

/**
 * Cycles a column between its two directions, starting from the direction that
 * reads as most useful for that column the first time it is clicked.
 */
export function nextDataTableSort<Field extends string, Sort extends string>(
  sort: Sort,
  field: Field,
  defaultColumnSort: Readonly<Record<Field, Sort>>,
): Sort {
  const ascending = field as string as Sort;
  const descending = `-${field}` as Sort;
  if (sort === ascending) return descending;
  if (sort === descending) return ascending;
  return defaultColumnSort[field];
}

function moveColumnToIndex<Field extends string>(
  order: readonly Field[],
  field: Field,
  targetIndex: number,
): Field[] {
  const sourceIndex = order.indexOf(field);
  if (sourceIndex < 0) return [...order];
  const next = order.filter((item) => item !== field);
  next.splice(Math.max(0, Math.min(targetIndex, next.length)), 0, field);
  return next;
}

function reorderColumnsForDrop<Field extends string>(
  order: readonly Field[],
  source: Field,
  target: Field,
  position: DataTableColumnDropPosition,
): Field[] {
  if (source === target) return [...order];
  const withoutSource = order.filter((field) => field !== source);
  const targetIndex = withoutSource.indexOf(target);
  if (targetIndex < 0) return [...order];
  const insertionIndex = targetIndex + (position === "after" ? 1 : 0);
  withoutSource.splice(insertionIndex, 0, source);
  return withoutSource;
}

/**
 * Owns column order, widths, and the freeze boundary, plus the drag and resize
 * interactions. `storageKey` scopes the persisted layout to one table; bump its
 * version suffix when a change makes stored layouts unreadable.
 */
export function useDataTableColumns<Field extends string>(
  columns: readonly DataTableColumn<Field>[],
  storageKey: string,
) {
  const byField = useMemo(
    () => new Map(columns.map((column) => [column.field, column])),
    [columns],
  );

  const defaults = useMemo<DataTableColumnPreferences<Field>>(
    () => ({
      order: columns.map((column) => column.field),
      frozenThrough: null,
      widths: Object.fromEntries(
        columns.map((column) => [column.field, column.defaultWidth]),
      ) as Readonly<Record<Field, number>>,
    }),
    [columns],
  );

  const [preferences, setPreferences] = useState(() =>
    loadColumnPreferences(storageKey, defaults, COLUMN_WIDTH_BOUNDS),
  );
  const [draggingColumn, setDraggingColumn] = useState<Field | null>(null);
  const [dropTarget, setDropTarget] = useState<ColumnDropTarget<Field> | null>(
    null,
  );
  const [resizingColumn, setResizingColumn] = useState<Field | null>(null);
  const [announcement, setAnnouncement] = useState("");
  const draggingRef = useRef<Field | null>(null);
  const detachResizeRef = useRef<(() => void) | null>(null);
  const tableRef = useRef<HTMLTableElement>(null);

  const { order, frozenThrough, widths } = preferences;
  const frozenFields = frozenColumnFields(order, frozenThrough);

  const getColumn = useCallback(
    (field: Field): DataTableColumn<Field> => {
      const column = byField.get(field);
      if (!column) throw new Error(`Unknown data table column: ${field}`);
      return column;
    },
    [byField],
  );

  const isColumnField = useCallback(
    (value: unknown): value is Field => byField.has(value as Field),
    [byField],
  );

  useEffect(() => {
    saveColumnPreferences(storageKey, preferences, defaults);
  }, [defaults, preferences, storageKey]);

  // Drop a resize still in flight if the table unmounts under it.
  useEffect(() => () => detachResizeRef.current?.(), []);

  const applyOrder = useCallback(
    (nextOrder: readonly Field[], movedField: Field) => {
      setPreferences((previous) => ({ ...previous, order: nextOrder }));
      setAnnouncement(
        `${getColumn(movedField).label} column moved to position ${String(
          nextOrder.indexOf(movedField) + 1,
        )} of ${String(nextOrder.length)}`,
      );
    },
    [getColumn],
  );

  const commitWidth = useCallback(
    (field: Field, width: number) => {
      setPreferences((previous) => ({
        ...previous,
        widths: { ...previous.widths, [field]: width },
      }));
      setAnnouncement(
        `${getColumn(field).label} column resized to ${String(width)} pixels`,
      );
    },
    [getColumn],
  );

  const clearDragState = useCallback(() => {
    draggingRef.current = null;
    setDraggingColumn(null);
    setDropTarget(null);
  }, []);

  const onDragStart = useCallback(
    (field: Field, event: React.DragEvent<HTMLButtonElement>) => {
      draggingRef.current = field;
      setDraggingColumn(field);
      setDropTarget(null);
      event.dataTransfer.effectAllowed = "move";
      event.dataTransfer.setData("text/plain", field);
    },
    [],
  );

  const dropPositionForEvent = (
    event: React.DragEvent<HTMLTableCellElement>,
  ): DataTableColumnDropPosition => {
    const bounds = event.currentTarget.getBoundingClientRect();
    return event.clientX < bounds.left + bounds.width / 2 ? "before" : "after";
  };

  const onDragOver = useCallback(
    (field: Field, event: React.DragEvent<HTMLTableCellElement>) => {
      const source = draggingRef.current;
      if (!source || source === field) return;
      event.preventDefault();
      event.dataTransfer.dropEffect = "move";
      const next: ColumnDropTarget<Field> = {
        field,
        position: dropPositionForEvent(event),
      };
      setDropTarget((current) =>
        current?.field === next.field && current.position === next.position
          ? current
          : next,
      );
    },
    [],
  );

  const onDrop = useCallback(
    (field: Field, event: React.DragEvent<HTMLTableCellElement>) => {
      const transferred = event.dataTransfer.getData("text/plain");
      const source =
        draggingRef.current ?? (isColumnField(transferred) ? transferred : null);
      if (!source || source === field) {
        clearDragState();
        return;
      }

      event.preventDefault();
      const position =
        dropTarget?.field === field
          ? dropTarget.position
          : dropPositionForEvent(event);
      applyOrder(reorderColumnsForDrop(order, source, field, position), source);
      clearDragState();
    },
    [applyOrder, clearDragState, dropTarget, isColumnField, order],
  );

  const onMoveByKeyboard = useCallback(
    (field: Field, key: ColumnMoveKey) => {
      const sourceIndex = order.indexOf(field);
      const targetIndex =
        key === "Home"
          ? 0
          : key === "End"
            ? order.length - 1
            : key === "ArrowLeft"
              ? Math.max(0, sourceIndex - 1)
              : Math.min(order.length - 1, sourceIndex + 1);
      if (sourceIndex < 0 || sourceIndex === targetIndex) return;
      applyOrder(moveColumnToIndex(order, field, targetIndex), field);
    },
    [applyOrder, order],
  );

  const onToggleFreeze = useCallback((field: Field) => {
    setPreferences((previous) => ({
      ...previous,
      frozenThrough: previous.frozenThrough === field ? null : field,
    }));
  }, []);

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
  const onResizeStart = useCallback(
    (field: Field, event: React.PointerEvent<HTMLDivElement>) => {
      if (event.button !== 0) return;
      // Keep the pointer press off the header's sort button and drag handle.
      event.preventDefault();
      event.stopPropagation();

      const startWidth = widths[field];
      const resize = { startX: event.clientX, width: startWidth };
      setResizingColumn(field);

      function handleMove(moveEvent: PointerEvent) {
        resize.width = clampColumnWidth(
          startWidth + moveEvent.clientX - resize.startX,
          MIN_COLUMN_WIDTH,
          MAX_COLUMN_WIDTH,
        );
        tableRef.current?.style.setProperty(
          columnWidthVar(field),
          `${String(resize.width)}px`,
        );
      }

      function handleEnd() {
        detachResize();
        setResizingColumn(null);
        if (resize.width !== startWidth) commitWidth(field, resize.width);
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
    },
    [commitWidth, widths],
  );

  const onResizeByKeyboard = useCallback(
    (field: Field, key: ColumnResizeKey) => {
      const current = widths[field];
      const next =
        key === "Home"
          ? getColumn(field).defaultWidth
          : clampColumnWidth(
              current +
                (key === "ArrowRight" ? COLUMN_RESIZE_STEP : -COLUMN_RESIZE_STEP),
              MIN_COLUMN_WIDTH,
              MAX_COLUMN_WIDTH,
            );
      if (next === current) return;
      commitWidth(field, next);
    },
    [commitWidth, getColumn, widths],
  );

  const onResizeReset = useCallback(
    (field: Field) => {
      const { defaultWidth } = getColumn(field);
      if (widths[field] === defaultWidth) return;
      commitWidth(field, defaultWidth);
    },
    [commitWidth, getColumn, widths],
  );

  const resetLayout = useCallback(() => {
    setPreferences(defaults);
    setAnnouncement("Column layout reset");
  }, [defaults]);

  /** Reorder/resize/freeze props for one header; the caller adds the sort props. */
  const headerProps = useCallback(
    (field: Field) => ({
      column: getColumn(field),
      frozen: frozenFields.includes(field),
      lastFrozen: frozenThrough === field,
      stickyLeft: stickyColumnLeft(frozenFields, field),
      width: widths[field],
      resizing: resizingColumn === field,
      dragging: draggingColumn === field,
      dropPosition:
        dropTarget?.field === field ? dropTarget.position : undefined,
      onDragStart,
      onDragOver,
      onDrop,
      onDragEnd: clearDragState,
      onMoveByKeyboard,
      onToggleFreeze,
      onResizeStart,
      onResizeByKeyboard,
      onResizeReset,
    }),
    [
      clearDragState,
      draggingColumn,
      dropTarget,
      frozenFields,
      frozenThrough,
      getColumn,
      onDragOver,
      onDragStart,
      onDrop,
      onMoveByKeyboard,
      onResizeByKeyboard,
      onResizeReset,
      onResizeStart,
      onToggleFreeze,
      resizingColumn,
      widths,
    ],
  );

  /** Sticky positioning props for one body cell. */
  const cellProps = useCallback(
    (field: Field) => {
      const column = getColumn(field);
      const frozen = frozenFields.includes(field);
      return {
        "data-column": field,
        "data-frozen": frozen || undefined,
        "data-frozen-edge": (frozenThrough === field) || undefined,
        style: frozen
          ? { left: stickyColumnLeft(frozenFields, field) }
          : undefined,
        className: cn(
          column.cellClassName,
          // Opaque base so scrolled columns stay hidden; the row hover tint
          // rides on top as an overlay so a frozen cell reads the same as the
          // rest of its row.
          frozen &&
            "sticky z-20 bg-card after:pointer-events-none after:absolute after:inset-0 after:transition-colors after:duration-300 after:content-[''] group-hover/row:after:bg-muted/45",
          frozenThrough === field && FROZEN_EDGE_CLASS,
        ),
      };
    },
    [frozenFields, frozenThrough, getColumn],
  );

  return {
    order,
    widths,
    announcement,
    resizingColumn,
    tableRef,
    /** `min-width` + the per-column width variables the table lays itself out on. */
    tableStyle: {
      minWidth: sumOfColumnWidths(order),
      ...columnWidthVars(order, widths),
    } as React.CSSProperties,
    columnStyle: (field: Field) => ({ width: `var(${columnWidthVar(field)})` }),
    isDefaultLayout: isDefaultColumnLayout(preferences, defaults),
    resetLayout,
    getColumn,
    headerProps,
    cellProps,
  };
}

export interface DataTableColumnHeaderProps<
  Field extends string,
  Sort extends string,
> {
  readonly column: DataTableColumn<Field>;
  readonly sort: Sort;
  readonly nextSort: Sort;
  readonly disabled: boolean;
  readonly frozen: boolean;
  readonly lastFrozen: boolean;
  readonly stickyLeft: string | undefined;
  readonly width: number;
  readonly resizing: boolean;
  readonly dragging: boolean;
  readonly dropPosition: DataTableColumnDropPosition | undefined;
  readonly onSort: (sort: Sort) => void;
  readonly onDragStart: (
    field: Field,
    event: React.DragEvent<HTMLButtonElement>,
  ) => void;
  readonly onDragOver: (
    field: Field,
    event: React.DragEvent<HTMLTableCellElement>,
  ) => void;
  readonly onDrop: (
    field: Field,
    event: React.DragEvent<HTMLTableCellElement>,
  ) => void;
  readonly onDragEnd: () => void;
  readonly onMoveByKeyboard: (field: Field, key: ColumnMoveKey) => void;
  readonly onToggleFreeze: (field: Field) => void;
  readonly onResizeStart: (
    field: Field,
    event: React.PointerEvent<HTMLDivElement>,
  ) => void;
  readonly onResizeByKeyboard: (field: Field, key: ColumnResizeKey) => void;
  readonly onResizeReset: (field: Field) => void;
}

export function DataTableColumnHeader<
  Field extends string,
  Sort extends string,
>({
  column,
  sort,
  nextSort,
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
}: DataTableColumnHeaderProps<Field, Sort>) {
  const { field, label } = column;
  const direction = dataTableSortDirection(sort, field);
  const Icon =
    direction === "ascending"
      ? ArrowUp
      : direction === "descending"
        ? ArrowDown
        : ArrowUpDown;
  const nextDirection = nextSort.startsWith("-") ? "descending" : "ascending";

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
          onClick={() => onSort(nextSort)}
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
