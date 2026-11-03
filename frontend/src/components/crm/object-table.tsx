import { useMemo } from "react";
import { ChevronDown, ChevronUp } from "lucide-react";
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
import type { CrmField, CrmRecordRow, CrmTableQuery } from "@/schemas/crm";

export interface CrmObjectTableProps {
  readonly query: CrmTableQuery;
  readonly columns?: readonly string[];
  readonly ownerId?: string;
  readonly onSelectRecord?: (recordId: string) => void;
}

function fallbackCell(field: CrmField, row: CrmRecordRow) {
  if (field.type === "record") {
    const labels = row.links[field.key]?.map((link) => link.label) ?? [];
    return labels.length ? labels.join(", ") : "—";
  }
  const value = row.values[field.key];
  if (value === null || value === undefined || value === "") return "—";
  if (typeof value === "boolean") return value ? "Yes" : "No";
  if (field.type === "currency") {
    const amount =
      typeof value === "number"
        ? value
        : typeof value === "object" &&
            value !== null &&
            "amount_minor" in value &&
            typeof value.amount_minor === "number"
          ? value.amount_minor
          : null;
    if (amount !== null) {
      return new Intl.NumberFormat(undefined, {
        style: "currency",
        currency: field.currency,
      }).format(amount / 100);
    }
  }
  if (
    field.type === "number" &&
    field.format === "percent" &&
    typeof value === "number"
  ) {
    return new Intl.NumberFormat(undefined, { style: "percent" }).format(value);
  }
  if (field.type === "select" && typeof value === "string") {
    return (
      field.options.find((option) => option.value === value)?.label ?? value
    );
  }
  if (Array.isArray(value)) return value.join(", ");
  return String(value);
}

export function CrmObjectTable({
  query,
  columns,
  ownerId,
  onSelectRecord,
}: CrmObjectTableProps) {
  const result = useCrmTableQuery(query, ownerId);
  const fields = useMemo(() => {
    const object = result.data?.object;
    if (!object) return [];
    const selected = columns ?? query.select ?? object.default_columns;
    return selected
      .map((key) => object.fields.find((field) => field.key === key))
      .filter((field): field is CrmField => Boolean(field));
  }, [columns, query.select, result.data?.object]);

  if (result.isPending) {
    return (
      <p role="status" className="text-12 text-text-tertiary">
        Loading records...
      </p>
    );
  }
  if (result.isError || !result.data) {
    return (
      <p role="alert" className="text-12 text-destructive">
        Could not load records.
      </p>
    );
  }
  if (result.data.rows.length === 0) {
    return (
      <div className="rounded-lg border border-dashed border-border px-4 py-8 text-center text-12 text-text-tertiary">
        No {result.data.object.plural_label.toLocaleLowerCase()} match this
        view.
      </div>
    );
  }

  return (
    <div className="overflow-hidden rounded-xl border border-border/50 bg-card">
      <Table>
        <TableHeader>
          <TableRow>
            {fields.map((field) => (
              <TableHead key={field.key}>
                <span className="inline-flex items-center gap-1">
                  {field.label}
                  {query.sort?.field === field.key ? (
                    query.sort.direction === "asc" ? (
                      <ChevronUp aria-label="Sorted ascending" />
                    ) : (
                      <ChevronDown aria-label="Sorted descending" />
                    )
                  ) : null}
                </span>
              </TableHead>
            ))}
          </TableRow>
        </TableHeader>
        <TableBody>
          {result.data.rows.map((row) => (
            <TableRow
              key={row.id}
              className={onSelectRecord ? "cursor-pointer" : undefined}
              onClick={
                onSelectRecord ? () => onSelectRecord(row.id) : undefined
              }
            >
              {fields.map((field) => (
                <TableCell key={field.key}>
                  {fallbackCell(field, row)}
                </TableCell>
              ))}
            </TableRow>
          ))}
        </TableBody>
      </Table>
      <div className="flex items-center justify-between border-t border-border/50 px-3 py-2">
        <span className="text-11 text-text-tertiary">
          {result.data.total_count.toLocaleString()} total
        </span>
        {result.data.next_cursor ? (
          <Button size="sm" variant="outline" disabled>
            More records available
          </Button>
        ) : null}
      </div>
    </div>
  );
}
