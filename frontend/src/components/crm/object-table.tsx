import { useMemo, useState, type KeyboardEvent } from "react";
import {
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  ChevronUp,
  ListFilter,
} from "lucide-react";
import { CrmFilterControls } from "@/components/crm/filter-controls";
import { Button } from "@/components/ui/button";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useCrmTableQuery } from "@/hooks/use-crm";
import { isCrmSortableField, renderCrmCell } from "@/lib/crm/field-kinds";
import type {
  CrmField,
  CrmFilter,
  CrmRecordRow,
  CrmTableQuery,
} from "@/schemas/crm";

export interface CrmObjectTableProps {
  readonly query: CrmTableQuery;
  readonly columns?: readonly string[];
  readonly ownerId?: string;
  readonly onSelectRecord?: (recordId: string) => void;
}

function interactiveKey(
  event: KeyboardEvent<HTMLTableRowElement>,
  select: () => void,
) {
  if (event.key !== "Enter" && event.key !== " ") return;
  event.preventDefault();
  select();
}

function RecordCells({
  row,
  fields,
}: {
  readonly row: CrmRecordRow;
  readonly fields: readonly CrmField[];
}) {
  return fields.map((field) => (
    <TableCell key={field.key}>
      {renderCrmCell(field, row.values[field.key], row.links[field.key])}
    </TableCell>
  ));
}

function CrmObjectTableState({
  query,
  columns,
  ownerId,
  onSelectRecord,
}: CrmObjectTableProps) {
  const [filters, setFilters] = useState<CrmFilter[]>(() => [...query.filters]);
  const [sort, setSort] = useState(query.sort);
  const [cursors, setCursors] = useState<(string | undefined)[]>(() => [
    query.cursor,
  ]);
  const [showFilters, setShowFilters] = useState(query.filters.length > 0);
  const cursor = cursors.at(-1);
  const pageQuery: CrmTableQuery = { ...query, filters, sort, cursor };
  const result = useCrmTableQuery(pageQuery, ownerId);
  const fields = useMemo(() => {
    const object = result.data?.object;
    if (!object) return [];
    const selected = columns ?? query.select ?? object.default_columns;
    return selected
      .map((key) => object.fields.find((field) => field.key === key))
      .filter((field): field is CrmField => Boolean(field));
  }, [columns, query.select, result.data?.object]);

  function resetPage(nextFilters: CrmFilter[]) {
    setFilters(nextFilters);
    setCursors([undefined]);
  }

  function toggleSort(field: CrmField) {
    setSort((current) =>
      current?.field === field.key
        ? current.direction === "asc"
          ? { field: field.key, direction: "desc" }
          : undefined
        : { field: field.key, direction: "asc" },
    );
    setCursors([undefined]);
  }

  if (result.isPending && !result.data) {
    return (
      <p role="status" className="text-12 text-text-tertiary">
        Loading records...
      </p>
    );
  }
  if (result.isError || !result.data) {
    return (
      <div className="space-y-2" role="alert">
        <p className="text-12 text-destructive">Could not load records.</p>
        <Button
          type="button"
          size="sm"
          variant="outline"
          onClick={() => void result.refetch()}
        >
          Retry
        </Button>
      </div>
    );
  }

  const object = result.data.object;
  const pageSize = query.page_size ?? 50;
  const first = result.data.rows.length
    ? (cursors.length - 1) * pageSize + 1
    : 0;
  const last = first ? first + result.data.rows.length - 1 : 0;
  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <Button
          type="button"
          size="sm"
          variant={filters.length ? "secondary" : "outline"}
          aria-expanded={showFilters}
          onClick={() => setShowFilters((current) => !current)}
        >
          <ListFilter />
          Filters{filters.length ? ` (${filters.length})` : ""}
        </Button>
        <span className="text-11 tabular-nums text-text-tertiary">
          {result.data.total_count.toLocaleString()} total
        </span>
      </div>
      {showFilters ? (
        <CrmFilterControls
          object={object}
          filters={filters}
          ownerId={ownerId}
          onChange={resetPage}
        />
      ) : null}

      {result.data.rows.length === 0 ? (
        <div className="rounded-lg border border-dashed border-border px-4 py-8 text-center text-12 text-text-tertiary">
          No {object.plural_label.toLocaleLowerCase()} match this view.
        </div>
      ) : (
        <>
          <div className="hidden overflow-hidden rounded-xl border border-border/50 bg-card md:block">
            <Table>
              <TableHeader>
                <TableRow>
                  {fields.map((field) => (
                    <TableHead key={field.key}>
                      {isCrmSortableField(field) ? (
                        <button
                          type="button"
                          className="inline-flex items-center gap-1 rounded-sm outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
                          onClick={() => toggleSort(field)}
                        >
                          {field.label}
                          {sort?.field === field.key ? (
                            sort.direction === "asc" ? (
                              <ChevronUp aria-label="Sorted ascending" />
                            ) : (
                              <ChevronDown aria-label="Sorted descending" />
                            )
                          ) : null}
                        </button>
                      ) : (
                        field.label
                      )}
                    </TableHead>
                  ))}
                </TableRow>
              </TableHeader>
              <TableBody>
                {result.data.rows.map((row) => {
                  const select = onSelectRecord
                    ? () => onSelectRecord(row.id)
                    : undefined;
                  return (
                    <TableRow
                      key={row.id}
                      tabIndex={select ? 0 : undefined}
                      aria-label={select ? `Open ${object.label}` : undefined}
                      className={
                        select
                          ? "cursor-pointer focus-visible:outline-2 focus-visible:outline-ring"
                          : undefined
                      }
                      onClick={select}
                      onKeyDown={
                        select
                          ? (event) => interactiveKey(event, select)
                          : undefined
                      }
                    >
                      <RecordCells row={row} fields={fields} />
                    </TableRow>
                  );
                })}
              </TableBody>
            </Table>
          </div>

          <div className="space-y-2 md:hidden">
            {result.data.rows.map((row) => (
              <button
                key={row.id}
                type="button"
                disabled={!onSelectRecord}
                onClick={
                  onSelectRecord ? () => onSelectRecord(row.id) : undefined
                }
                className="block w-full space-y-2 rounded-lg border border-border/50 bg-card p-3 text-left disabled:opacity-100"
              >
                {fields.map((field) => (
                  <span
                    key={field.key}
                    className="grid grid-cols-[minmax(5rem,0.7fr)_minmax(0,1.3fr)] gap-3 text-12"
                  >
                    <span className="text-text-tertiary">{field.label}</span>
                    <span className="min-w-0 break-words text-foreground">
                      {renderCrmCell(
                        field,
                        row.values[field.key],
                        row.links[field.key],
                      )}
                    </span>
                  </span>
                ))}
              </button>
            ))}
          </div>
        </>
      )}

      <div className="flex items-center justify-between gap-3">
        <span className="text-11 tabular-nums text-text-tertiary">
          {first.toLocaleString()}-{last.toLocaleString()} of{" "}
          {result.data.total_count.toLocaleString()}
        </span>
        <div className="flex items-center gap-1">
          <Button
            type="button"
            size="icon"
            variant="outline"
            aria-label="Previous page"
            disabled={cursors.length <= 1 || result.isFetching}
            onClick={() => setCursors((current) => current.slice(0, -1))}
          >
            <ChevronLeft />
          </Button>
          <Button
            type="button"
            size="icon"
            variant="outline"
            aria-label="Next page"
            disabled={!result.data.next_cursor || result.isFetching}
            onClick={() => {
              const nextCursor = result.data?.next_cursor;
              if (nextCursor) setCursors((current) => [...current, nextCursor]);
            }}
          >
            <ChevronRight />
          </Button>
        </div>
      </div>
    </div>
  );
}

export function CrmObjectTable(props: CrmObjectTableProps) {
  return <CrmObjectTableState key={JSON.stringify(props.query)} {...props} />;
}
