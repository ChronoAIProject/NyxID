import { useCallback, useMemo, useRef, useState } from "react";
import { cn } from "@/lib/utils";
import { TableHead } from "@/components/ui/table";
import {
  ArrowDown,
  ArrowUp,
  ArrowUpDown,
  GripVertical,
  Pin,
  PinOff,
} from "lucide-react";

/**
 * Reorderable, freezable, sortable column headers for the admin data tables.
 *
 * Sort strings follow the server's convention throughout: `field` ascending,
 * `-field` descending.
 */
export interface DataTableColumn<Field extends string> {
  readonly field: Field;
  readonly label: string;
  readonly width: number;
  readonly cellClassName?: string;
}

export type DataTableColumnDropPosition = "before" | "after";

interface ColumnDropTarget<Field extends string> {
  readonly field: Field;
  readonly position: DataTableColumnDropPosition;
}

type ColumnMoveKey = "ArrowLeft" | "ArrowRight" | "Home" | "End";

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
 * Owns column order, the freeze boundary, and the drag interaction. Sorting
 * stays with the caller: it lives in the URL, not in component state.
 */
export function useDataTableColumns<Field extends string>(
  columns: readonly DataTableColumn<Field>[],
) {
  const byField = useMemo(
    () => new Map(columns.map((column) => [column.field, column])),
    [columns],
  );
  const defaultOrder = useMemo(
    () => columns.map((column) => column.field),
    [columns],
  );
  const totalWidth = useMemo(
    () => columns.reduce((width, column) => width + column.width, 0),
    [columns],
  );

  const [order, setOrder] = useState<readonly Field[]>(defaultOrder);
  const [frozenThrough, setFrozenThrough] = useState<Field | null>(null);
  const [draggingColumn, setDraggingColumn] = useState<Field | null>(null);
  const [dropTarget, setDropTarget] = useState<ColumnDropTarget<Field> | null>(
    null,
  );
  const [announcement, setAnnouncement] = useState("");
  const draggingRef = useRef<Field | null>(null);

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

  // Left offset of every frozen column, accumulated in render order so each one
  // sticks flush against the previous.
  const stickyOffsets = useMemo(() => {
    const offsets = new Map<Field, number>();
    if (!frozenThrough) return offsets;
    let left = 0;
    for (const field of order) {
      offsets.set(field, left);
      left += getColumn(field).width;
      if (field === frozenThrough) break;
    }
    return offsets;
  }, [frozenThrough, getColumn, order]);

  const applyOrder = useCallback(
    (nextOrder: readonly Field[], movedField: Field) => {
      setOrder(nextOrder);
      setAnnouncement(
        `${getColumn(movedField).label} column moved to position ${String(
          nextOrder.indexOf(movedField) + 1,
        )} of ${String(nextOrder.length)}`,
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
    setFrozenThrough((current) => (current === field ? null : field));
  }, []);

  /** Reorder/freeze/drag props for one header; the caller adds the sort props. */
  const headerProps = useCallback(
    (field: Field) => ({
      column: getColumn(field),
      frozen: stickyOffsets.has(field),
      lastFrozen: frozenThrough === field,
      stickyLeft: stickyOffsets.get(field),
      dragging: draggingColumn === field,
      dropPosition:
        dropTarget?.field === field ? dropTarget.position : undefined,
      onDragStart,
      onDragOver,
      onDrop,
      onDragEnd: clearDragState,
      onMoveByKeyboard,
      onToggleFreeze,
    }),
    [
      clearDragState,
      draggingColumn,
      dropTarget,
      frozenThrough,
      getColumn,
      onDragOver,
      onDragStart,
      onDrop,
      onMoveByKeyboard,
      onToggleFreeze,
      stickyOffsets,
    ],
  );

  /** Sticky positioning props for one body cell. */
  const cellProps = useCallback(
    (field: Field) => {
      const column = getColumn(field);
      const frozen = stickyOffsets.has(field);
      return {
        "data-column": field,
        "data-frozen": frozen || undefined,
        style: frozen ? { left: stickyOffsets.get(field) } : undefined,
        className: cn(
          column.cellClassName,
          // Opaque base so scrolled columns stay hidden; the row hover tint
          // rides on top as an overlay so a frozen cell reads the same as the
          // rest of its row.
          frozen &&
            "sticky z-20 bg-card after:pointer-events-none after:absolute after:inset-0 after:transition-colors after:duration-300 after:content-[''] group-hover/row:after:bg-muted/45",
          frozenThrough === field &&
            "border-r-2 border-border shadow-[3px_0_6px_-4px_rgba(0,0,0,0.35)]",
        ),
      };
    },
    [frozenThrough, getColumn, stickyOffsets],
  );

  return {
    order,
    announcement,
    totalWidth,
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
  readonly stickyLeft: number | undefined;
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
  dragging,
  dropPosition,
  onSort,
  onDragStart,
  onDragOver,
  onDrop,
  onDragEnd,
  onMoveByKeyboard,
  onToggleFreeze,
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
      style={frozen ? { left: stickyLeft } : undefined}
      onDragOver={(event) => onDragOver(field, event)}
      onDrop={(event) => onDrop(field, event)}
      className={cn(
        "group/header relative h-10 p-0 transition-colors",
        frozen && "sticky z-30 bg-card",
        lastFrozen &&
          "border-r-2 border-border shadow-[3px_0_6px_-4px_rgba(0,0,0,0.35)]",
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
          "flex h-full min-w-0 items-center",
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
    </TableHead>
  );
}
