import { useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  useCrmImport,
  useCrmExport,
  type CrmImportInput,
  type CrmImportResult,
} from "@/hooks/use-crm";
import { parseCrmImport } from "@/lib/crm/import";
import type { CrmObject, CrmTableQuery } from "@/schemas/crm";
function download(value: unknown, filename: string) {
  const url = URL.createObjectURL(
    new Blob([JSON.stringify(value, null, 2)], { type: "application/json" }),
  );
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = filename;
  anchor.click();
  URL.revokeObjectURL(url);
}
export function CrmImportExport({
  object,
  query,
  ownerId,
}: {
  readonly object: CrmObject;
  readonly query: CrmTableQuery;
  readonly ownerId?: string;
}) {
  const importer = useCrmImport(ownerId);
  const exporter = useCrmExport(ownerId);
  const [source, setSource] = useState("");
  const [rows, setRows] = useState<Record<string, unknown>[]>();
  const [mapping, setMapping] = useState<Record<string, string>>({});
  const [matchField, setMatchField] = useState(
    object.fields.find(
      (f) =>
        f.type === "text" && !f.archived && ["domain", "email"].includes(f.key),
    )?.key ??
      object.fields.find((f) => f.type === "text" && !f.archived)?.key ??
      "",
  );
  const [policy, setPolicy] = useState<CrmImportInput["policy"]>("create_only");
  const [preview, setPreview] = useState<CrmImportResult>();
  const [error, setError] = useState<string>();
  const [exportCursor, setExportCursor] = useState<string>();
  const actionKey = useRef<string | null>(null);
  const staged = useRef<CrmImportInput | undefined>(undefined);
  function resetPreview() {
    setPreview(undefined);
    actionKey.current = null;
    staged.current = undefined;
  }
  async function exportPage(cursor?: string) {
    setError(undefined);
    try {
      const result = await exporter.mutateAsync({
        ...query,
        cursor,
        page_size: 100,
      });
      const records = result.rows.map((record) => ({
        ...record.values,
        ...Object.fromEntries(
          Object.entries(record.links).map(([key, links]) => {
            const field = result.object.fields.find((f) => f.key === key);
            return [
              key,
              field?.type === "record" && field.cardinality === "many"
                ? links.map((l) => l.id)
                : (links[0]?.id ?? null),
            ];
          }),
        ),
      }));
      download(
        {
          object: result.object,
          records,
          total_count: result.total_count,
          next_cursor: result.next_cursor,
        },
        `${object.key}-export-page.json`,
      );
      setExportCursor(result.next_cursor ?? undefined);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Could not export.");
    }
  }
  return (
    <section
      aria-label="Import and export"
      className="space-y-4 rounded-lg border border-border p-4"
    >
      <div className="flex flex-wrap gap-2">
        <Button
          variant="outline"
          isLoading={exporter.isPending}
          onClick={() => void exportPage()}
        >
          Export first 100 matching records
        </Button>
        {exportCursor ? (
          <Button
            variant="outline"
            onClick={() => void exportPage(exportCursor)}
          >
            Export next page
          </Button>
        ) : null}
      </div>
      <p className="type-meta text-text-secondary">
        Exports use the displayed filters and typed raw values. Each download
        includes its schema, total and next cursor and can be imported directly
        (up to 100 records). JSON arrays of row objects are also accepted.
        Import is limited to 100 rows / 512 KB per preview.
      </p>
      {object.access !== "read" ? (
        <fieldset disabled={importer.isPending} className="space-y-3">
          <Label>
            CSV or JSON rows
            <Input
              type="file"
              accept=".csv,.json,text/csv,application/json"
              onChange={async (e) => {
                const file = e.target.files?.[0];
                if (!file) return;
                if (file.size > 512000) {
                  setError("Use a file of at most 512 KB.");
                  return;
                }
                setSource(await file.text());
                setRows(undefined);
                resetPreview();
              }}
            />
          </Label>
          <Textarea
            aria-label="Import source"
            value={source}
            onChange={(e) => {
              setSource(e.target.value);
              setRows(undefined);
              resetPreview();
            }}
            placeholder={"name,domain\nAcme,acme.example"}
          />
          <Button
            variant="outline"
            onClick={() => {
              try {
                setError(undefined);
                const parsed = parseCrmImport(source);
                setRows(parsed);
                setMapping(
                  Object.fromEntries(
                    [...new Set(parsed.flatMap((row) => Object.keys(row)))]
                      .filter((column) =>
                        object.fields.some(
                          (f) => f.key === column && !f.archived,
                        ),
                      )
                      .map((key) => [key, key]),
                  ),
                );
                resetPreview();
              } catch (cause) {
                setError(
                  cause instanceof Error ? cause.message : "Invalid source.",
                );
              }
            }}
          >
            Map columns
          </Button>
          {rows ? (
            <>
              <div className="grid gap-2 sm:grid-cols-2">
                {[...new Set(rows.flatMap((row) => Object.keys(row)))].map(
                  (column) => (
                    <Label key={column}>
                      {column}
                      <select
                        className="ml-2 rounded border border-border bg-background p-2"
                        value={mapping[column] ?? ""}
                        onChange={(e) => {
                          setMapping((m) => ({
                            ...m,
                            [column]: e.target.value,
                          }));
                          resetPreview();
                        }}
                      >
                        <option value="">Ignore column</option>
                        {object.fields
                          .filter((f) => !f.archived)
                          .map((field) => (
                            <option key={field.key} value={field.key}>
                              {field.label} ({field.type})
                            </option>
                          ))}
                      </select>
                    </Label>
                  ),
                )}
              </div>
              <Label>
                Matching identity
                <select
                  className="ml-2 rounded border border-border bg-background p-2"
                  value={matchField}
                  onChange={(e) => {
                    setMatchField(e.target.value);
                    resetPreview();
                  }}
                >
                  {object.fields
                    .filter((f) => f.type === "text" && !f.archived)
                    .map((f) => (
                      <option key={f.key} value={f.key}>
                        {f.label}
                      </option>
                    ))}
                </select>
              </Label>
              <Label>
                Existing record policy
                <select
                  className="ml-2 rounded border border-border bg-background p-2"
                  value={policy}
                  onChange={(e) => {
                    setPolicy(e.target.value as CrmImportInput["policy"]);
                    resetPreview();
                  }}
                >
                  <option value="create_only">
                    Reject existing identities
                  </option>
                  <option value="update_or_create">
                    Update existing, create missing
                  </option>
                  <option value="skip_existing">
                    Skip existing, create missing
                  </option>
                </select>
              </Label>
              <p className="type-meta text-text-secondary">
                Matching trims whitespace and ignores case; ambiguous matches
                and repeated identities are rejected. Relationship values must
                be exact authorized record IDs; select values use stable option
                keys. Currency strings are decimal amounts; percentage numbers
                are fractions. Missing JSON keys preserve existing values;
                explicit null or blank clears an optional field.
              </p>
              <Button
                onClick={async () => {
                  setError(undefined);
                  try {
                    const input: CrmImportInput = {
                      object: object.key,
                      rows,
                      mapping: Object.fromEntries(
                        Object.entries(mapping).filter(([, key]) => key),
                      ),
                      match_field: matchField,
                      policy,
                    };
                    const result = await importer.mutateAsync({
                      input,
                      commit: false,
                    });
                    staged.current = { ...input, snapshot: result.snapshot };
                    setPreview(result);
                    actionKey.current = null;
                  } catch (cause) {
                    setError(
                      cause instanceof Error
                        ? cause.message
                        : "Preview failed.",
                    );
                  }
                }}
              >
                Preview import
              </Button>
            </>
          ) : null}
          {preview ? (
            <div className="space-y-2">
              <p className="type-group-label">
                {preview.rows.length} rows ·{" "}
                {preview.rows.filter((r) => r.action === "create").length}{" "}
                create ·{" "}
                {preview.rows.filter((r) => r.action === "update").length}{" "}
                update ·{" "}
                {preview.rows.filter((r) => r.action === "skip").length} skip
              </p>
              <ol className="max-h-64 overflow-auto type-meta">
                {preview.rows.map((row) => (
                  <li key={row.index}>
                    Row {row.index + 1}: {row.action}
                    {row.errors
                      .map((e) => ` · ${e.path}: ${e.message}`)
                      .join("")}
                  </li>
                ))}
              </ol>
              {preview.committed ? (
                <p role="status">Import committed.</p>
              ) : (
                <Button
                  disabled={preview.rows.some((r) => r.errors.length > 0)}
                  isLoading={importer.isPending}
                  onClick={async () => {
                    if (!staged.current) return;
                    setError(undefined);
                    actionKey.current ??= crypto.randomUUID();
                    try {
                      setPreview(
                        await importer.mutateAsync({
                          input: staged.current,
                          commit: true,
                          idempotencyKey: actionKey.current,
                        }),
                      );
                    } catch (cause) {
                      setError(
                        cause instanceof Error
                          ? cause.message
                          : "Commit failed. Preview again if records changed.",
                      );
                    }
                  }}
                >
                  Commit reviewed import
                </Button>
              )}
            </div>
          ) : null}
        </fieldset>
      ) : null}
      {error ? (
        <p role="alert" className="text-12 text-destructive">
          {error}
        </p>
      ) : null}
    </section>
  );
}
