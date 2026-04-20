import { useMemo } from "react";
import { Plus, Trash2, Info } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Checkbox } from "@/components/ui/checkbox";
import { Badge } from "@/components/ui/badge";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import {
  MAX_DEFAULT_HEADERS,
  type DefaultRequestHeader,
  isDenylistedHeaderName,
} from "@/schemas/default-request-headers";

/**
 * Shared editor for service-level default request headers (NyxID#356).
 *
 * Admin and per-user surfaces both use this. The editor is "dumb": the
 * parent owns the header list and any validation errors. Validation in
 * this file is lightweight client-side UX hints (denylist + duplicate
 * names) — the authoritative Zod schema validates the full list on
 * submit.
 *
 * `value` is always an array of rows. To clear all headers, the parent
 * sends an empty array to the mutation (the API hook translates
 * "was-non-empty -> now empty" into the tri-state `null` on the wire so
 * the backend explicit-clears the stored list).
 */

export interface DefaultHeadersEditorProps {
  readonly value: ReadonlyArray<DefaultRequestHeader>;
  readonly onChange: (next: ReadonlyArray<DefaultRequestHeader>) => void;
  readonly disabled?: boolean;
  readonly readOnly?: boolean;
  /** When true, appends "(from catalog)" badges to the value badge. */
  readonly fromCatalog?: boolean;
  /** Per-row error messages, keyed by row index. */
  readonly errors?: Readonly<Record<number, string>>;
}

function emptyRow(): DefaultRequestHeader {
  return { name: "", value: "", overridable: false, sensitive: false };
}

export function DefaultHeadersEditor({
  value,
  onChange,
  disabled = false,
  readOnly = false,
  fromCatalog = false,
  errors,
}: DefaultHeadersEditorProps) {
  const canAdd = !readOnly && !disabled && value.length < MAX_DEFAULT_HEADERS;

  const rowWarnings = useMemo<ReadonlyArray<string | null>>(() => {
    const seen = new Map<string, number>();
    return value.map((row, idx) => {
      if (errors?.[idx]) return errors[idx];
      const trimmed = row.name.trim();
      if (trimmed.length === 0) {
        return null;
      }
      if (isDenylistedHeaderName(trimmed)) {
        return "Reserved header name";
      }
      const key = trimmed.toLowerCase();
      const prior = seen.get(key);
      if (prior !== undefined) {
        return `Duplicate of row ${String(prior + 1)}`;
      }
      seen.set(key, idx);
      return null;
    });
  }, [value, errors]);

  function updateRow(idx: number, patch: Partial<DefaultRequestHeader>) {
    onChange(value.map((row, i) => (i === idx ? { ...row, ...patch } : row)));
  }

  function removeRow(idx: number) {
    onChange(value.filter((_, i) => i !== idx));
  }

  function addRow() {
    onChange([...value, emptyRow()]);
  }

  if (readOnly) {
    return <ReadOnlyHeadersList value={value} fromCatalog={fromCatalog} />;
  }

  return (
    <div className="space-y-2">
      {value.length === 0 ? (
        <div className="rounded-[10px] border border-dashed border-border p-4 text-center text-xs text-muted-foreground">
          No default headers configured.
        </div>
      ) : (
        <div className="rounded-[10px] border border-border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="w-[28%]">Name</TableHead>
                <TableHead className="w-[38%]">Value</TableHead>
                <TableHead className="w-[12%]">
                  <FlagHeader
                    label="Overridable"
                    hint="If checked, a caller-supplied value wins. Default: the admin value always wins."
                  />
                </TableHead>
                <TableHead className="w-[12%]">
                  <FlagHeader
                    label="Sensitive"
                    hint="Redact the value in API responses. v1 stores values plaintext — do not use for real secrets. Use the service auth method instead."
                  />
                </TableHead>
                <TableHead className="w-[10%] text-right" />
              </TableRow>
            </TableHeader>
            <TableBody>
              {value.map((row, idx) => {
                const warning = rowWarnings[idx];
                return (
                  <TableRow key={idx}>
                    <TableCell className="align-top">
                      <Input
                        aria-label={`Header ${String(idx + 1)} name`}
                        value={row.name}
                        onChange={(e) => updateRow(idx, { name: e.target.value })}
                        placeholder="X-Api-Version"
                        maxLength={256}
                        disabled={disabled}
                        className="font-mono text-xs"
                      />
                      {warning && (
                        <p className="mt-1 text-[11px] text-destructive">
                          {warning}
                        </p>
                      )}
                    </TableCell>
                    <TableCell className="align-top">
                      <Input
                        aria-label={`Header ${String(idx + 1)} value`}
                        value={row.value}
                        onChange={(e) => updateRow(idx, { value: e.target.value })}
                        placeholder="v2"
                        maxLength={4096}
                        disabled={disabled}
                        type={row.sensitive ? "password" : "text"}
                        className="font-mono text-xs"
                      />
                    </TableCell>
                    <TableCell className="align-top">
                      <div className="flex h-9 items-center">
                        <Checkbox
                          aria-label={`Header ${String(idx + 1)} overridable`}
                          checked={row.overridable}
                          onCheckedChange={(checked) =>
                            updateRow(idx, { overridable: checked === true })
                          }
                          disabled={disabled}
                        />
                      </div>
                    </TableCell>
                    <TableCell className="align-top">
                      <div className="flex h-9 items-center">
                        <Checkbox
                          aria-label={`Header ${String(idx + 1)} sensitive`}
                          checked={row.sensitive}
                          onCheckedChange={(checked) =>
                            updateRow(idx, { sensitive: checked === true })
                          }
                          disabled={disabled}
                        />
                      </div>
                    </TableCell>
                    <TableCell className="align-top text-right">
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        onClick={() => removeRow(idx)}
                        disabled={disabled}
                        aria-label={`Remove header ${String(idx + 1)}`}
                      >
                        <Trash2 className="h-4 w-4" />
                      </Button>
                    </TableCell>
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
        </div>
      )}

      <div className="flex items-center justify-between">
        <p className="text-[11px] text-muted-foreground">
          {value.length} / {MAX_DEFAULT_HEADERS} headers. Values stored in
          plaintext; do not place real secrets here.
        </p>
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={addRow}
          disabled={!canAdd}
        >
          <Plus className="mr-1 h-3 w-3" />
          Add header
        </Button>
      </div>
    </div>
  );
}

function FlagHeader({
  label,
  hint,
}: {
  readonly label: string;
  readonly hint: string;
}) {
  return (
    <span className="inline-flex items-center gap-1">
      {label}
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            type="button"
            className="inline-flex h-4 w-4 items-center justify-center rounded-full text-muted-foreground transition-colors hover:text-foreground"
            aria-label={`${label} info`}
          >
            <Info className="h-3 w-3" />
          </button>
        </TooltipTrigger>
        <TooltipContent className="max-w-[260px] text-xs">
          {hint}
        </TooltipContent>
      </Tooltip>
    </span>
  );
}

/**
 * Read-only rendering used for the inherited "(from catalog)" section
 * and for surfaces where the caller lacks write permission.
 */
function ReadOnlyHeadersList({
  value,
  fromCatalog,
}: {
  readonly value: ReadonlyArray<DefaultRequestHeader>;
  readonly fromCatalog: boolean;
}) {
  if (value.length === 0) {
    return (
      <p className="text-xs text-muted-foreground">
        {fromCatalog
          ? "No admin-configured defaults for this service."
          : "No default headers configured."}
      </p>
    );
  }
  return (
    <div className="rounded-[10px] border border-border">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead className="w-[28%]">Name</TableHead>
            <TableHead className="w-[52%]">Value</TableHead>
            <TableHead className="w-[20%]">Flags</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {value.map((row, idx) => (
            <TableRow key={idx}>
              <TableCell className="font-mono text-xs">{row.name}</TableCell>
              <TableCell className="font-mono text-xs">
                {row.sensitive ? (
                  <span className="text-muted-foreground">{"\u2022\u2022\u2022\u2022\u2022"}</span>
                ) : (
                  row.value
                )}
              </TableCell>
              <TableCell className="space-x-1">
                {row.overridable && (
                  <Badge variant="outline" className="text-[10px]">
                    overridable
                  </Badge>
                )}
                {row.sensitive && (
                  <Badge variant="outline" className="text-[10px]">
                    sensitive
                  </Badge>
                )}
                {fromCatalog && (
                  <Badge variant="secondary" className="text-[10px]">
                    from catalog
                  </Badge>
                )}
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </div>
  );
}
