import { useEffect, useMemo, useState } from "react";
import { Check, ChevronsUpDown, Search, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { useCrmObject, useCrmTableQuery } from "@/hooks/use-crm";
import type { CrmField, CrmRecordLink } from "@/schemas/crm";
import { useAuthStore } from "@/stores/auth-store";

type RecordField = Extract<CrmField, { type: "record" }>;

export function CrmRecordPicker({
  field,
  value,
  selectedLinks = [],
  ownerId,
  disabled,
  onChange,
  id,
  describedBy,
}: {
  readonly field: RecordField;
  readonly value: string | readonly string[] | null;
  readonly selectedLinks?: readonly CrmRecordLink[];
  readonly ownerId?: string;
  readonly disabled?: boolean;
  readonly onChange: (value: string | string[] | null) => void;
  readonly id?: string;
  readonly describedBy?: string;
}) {
  const [open, setOpen] = useState(false);
  const actorId = useAuthStore((state) => state.user?.id);
  const scope = `${actorId ?? "anonymous"}:${ownerId ?? "personal"}:${field.target}`;
  const [chosen, setChosen] = useState<{
    scope: string;
    labels: Map<string, string>;
  }>({ scope, labels: new Map() });
  const [search, setSearch] = useState("");
  const [queryText, setQueryText] = useState("");
  const [cursors, setCursors] = useState<(string | undefined)[]>([undefined]);
  const object = useCrmObject(field.target, ownerId, open);
  const titleField = object.data?.fields.find(
    (candidate) => candidate.type === "text" && candidate.required,
  );
  useEffect(() => {
    if (search.trim() === queryText) return;
    const timer = window.setTimeout(() => {
      setQueryText(search.trim());
      setCursors([undefined]);
    }, 200);
    return () => window.clearTimeout(timer);
  }, [queryText, search]);
  const records = useCrmTableQuery(
    open
      ? {
          object: field.target,
          select: titleField ? [titleField.key] : undefined,
          filters:
            queryText && titleField
              ? [{ field: titleField.key, op: "contains", value: queryText }]
              : [],
          sort: titleField
            ? { field: titleField.key, direction: "asc" }
            : undefined,
          page_size: 50,
          cursor: cursors.at(-1),
          include_archived: false,
        }
      : undefined,
    ownerId,
  );
  const selectedIds = useMemo(
    () =>
      new Set(
        Array.isArray(value)
          ? value
          : typeof value === "string" && value
            ? [value]
            : [],
      ),
    [value],
  );
  const labels = new Map<string, string>([
    ...(chosen.scope === scope ? chosen.labels : new Map<string, string>()),
    ...selectedLinks.map((link): [string, string] => [
      link.id,
      link.archived && !link.label.includes("(archived)")
        ? `${link.label} (archived)`
        : link.label,
    ]),
  ]);
  for (const row of records.data?.rows ?? []) {
    if (titleField)
      labels.set(row.id, String(row.values[titleField.key] ?? "Untitled"));
  }
  const summary =
    selectedIds.size === 0
      ? `Select ${object.data?.label?.toLocaleLowerCase() ?? "record"}`
      : [...selectedIds].map((id) => labels.get(id) ?? id).join(", ");

  function choose(id: string) {
    const nextIds =
      field.cardinality === "one"
        ? [id]
        : selectedIds.has(id)
          ? [...selectedIds].filter((value) => value !== id)
          : [...selectedIds, id];
    setChosen({
      scope,
      labels: new Map(
        nextIds.slice(0, 100).map((id) => [id, labels.get(id) ?? "Record"]),
      ),
    });
    if (field.cardinality === "one") {
      onChange(id);
      setOpen(false);
      return;
    }
    onChange(
      selectedIds.has(id)
        ? [...selectedIds].filter((candidate) => candidate !== id)
        : [...selectedIds, id],
    );
  }

  return (
    <Popover
      open={open && !disabled}
      onOpenChange={(next) => {
        setOpen(next);
        if (!next) setSearch("");
      }}
    >
      <PopoverTrigger asChild>
        <Button
          id={id}
          aria-describedby={describedBy}
          type="button"
          variant="outline"
          role="combobox"
          aria-label={field.label}
          aria-expanded={open}
          disabled={disabled}
          className="h-auto min-h-8 w-full justify-between text-left font-normal"
        >
          <span
            className={
              selectedIds.size ? "min-w-0 truncate" : "text-text-tertiary"
            }
          >
            {summary}
          </span>
          <ChevronsUpDown aria-hidden="true" />
        </Button>
      </PopoverTrigger>
      <PopoverContent
        align="start"
        className="w-[var(--radix-popover-trigger-width)] min-w-64 space-y-2 p-2"
      >
        <div className="relative">
          <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 text-text-tertiary" />
          <Input
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            aria-label={`Search ${object.data?.plural_label ?? "records"}`}
            placeholder={`Search ${object.data?.plural_label?.toLocaleLowerCase() ?? "records"}...`}
            className="pl-8"
          />
        </div>
        <div
          className="max-h-64 overflow-y-auto"
          role="listbox"
          aria-multiselectable={field.cardinality === "many"}
        >
          {records.isFetching ? (
            <p role="status" className="p-3 text-11 text-text-tertiary">
              Searching...
            </p>
          ) : null}
          {records.isError ? (
            <p role="alert" className="p-3 text-11 text-destructive">
              Could not load records.
            </p>
          ) : null}
          {records.data?.rows.map((row) => {
            const selected = selectedIds.has(row.id);
            const label = titleField
              ? String(row.values[titleField.key] ?? "Untitled")
              : row.id;
            return (
              <button
                key={row.id}
                type="button"
                role="option"
                aria-selected={selected}
                onClick={() => choose(row.id)}
                className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-12 hover:bg-overlay-strong"
              >
                {field.cardinality === "many" ? (
                  <Checkbox
                    checked={selected}
                    tabIndex={-1}
                    aria-hidden="true"
                  />
                ) : (
                  <span className="flex size-4 items-center justify-center">
                    {selected ? <Check aria-hidden="true" /> : null}
                  </span>
                )}
                <span className="min-w-0 flex-1 truncate">{label}</span>
              </button>
            );
          })}
          {records.data?.rows.length === 0 ? (
            <p className="p-3 text-11 text-text-tertiary">
              No matching records.
            </p>
          ) : null}
        </div>
        <div className="flex justify-between gap-2">
          {cursors.length > 1 ? (
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={records.isFetching}
              onClick={() => setCursors((current) => current.slice(0, -1))}
            >
              Previous results
            </Button>
          ) : null}
          {records.data?.next_cursor ? (
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={records.isFetching}
              onClick={() =>
                setCursors((current) => [
                  ...current,
                  records.data?.next_cursor ?? undefined,
                ])
              }
            >
              Next results
            </Button>
          ) : null}
          {records.isError ? (
            <Button
              type="button"
              size="sm"
              variant="outline"
              onClick={() => void records.refetch()}
            >
              Retry results
            </Button>
          ) : null}
        </div>
        <div className="flex items-center justify-between border-t border-border/50 pt-2">
          <Button
            type="button"
            size="sm"
            variant="ghost"
            disabled={selectedIds.size === 0}
            onClick={() => onChange(field.cardinality === "many" ? [] : null)}
          >
            <X />
            Clear
          </Button>
          {field.cardinality === "many" ? (
            <Button
              type="button"
              size="sm"
              variant="outline"
              onClick={() => setOpen(false)}
            >
              Done
            </Button>
          ) : null}
        </div>
      </PopoverContent>
    </Popover>
  );
}
