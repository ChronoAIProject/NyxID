import { useState } from "react";
import { useCrmAggregate } from "@/hooks/use-crm";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Input } from "@/components/ui/input";
import type { CrmObject, CrmTableQuery } from "@/schemas/crm";
export function CrmDateReport({
  object,
  query,
  ownerId,
}: {
  readonly object: CrmObject;
  readonly query: CrmTableQuery;
  readonly ownerId?: string;
}) {
  const dates = object.fields.filter(
    (field) => !field.archived && ["date", "datetime"].includes(field.type),
  );
  const [fieldKey, setFieldKey] = useState(dates[0]?.key ?? "");
  const [unit, setUnit] = useState<"day" | "week" | "month">("month");
  const [timezone, setTimezone] = useState(
    Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC",
  );
  const field = dates.find((item) => item.key === fieldKey);
  const result = useCrmAggregate(
    field
      ? {
          object: object.key,
          filters: query.filters,
          group_by: field.key,
          date_bucket: {
            unit,
            timezone: field.type === "date" ? "UTC" : timezone,
          },
          measure: { op: "count" },
          order: "label_asc",
        }
      : undefined,
    ownerId,
    Boolean(field),
  );
  return (
    <section
      aria-label="Dated report"
      className="space-y-3 rounded-lg border border-border p-4"
    >
      <h3 className="text-13 font-semibold">Records by date</h3>
      <p className="text-12 text-text-secondary">
        Counts use current record values and exclude archived records. Calendar
        dates retain their written date in UTC; datetime values use the selected
        IANA timezone. Weeks start Monday. This report does not reconstruct past
        pipeline state.
      </p>
      {dates.length ? (
        <>
          <div className="flex flex-wrap items-center gap-3">
            <Label>
              Date field
              <select
                className="ml-2 rounded border border-border bg-background p-2"
                value={fieldKey}
                onChange={(e) => setFieldKey(e.target.value)}
              >
                {dates.map((item) => (
                  <option key={item.key} value={item.key}>
                    {item.label}
                  </option>
                ))}
              </select>
            </Label>
            <Label>
              Bucket
              <select
                className="ml-2 rounded border border-border bg-background p-2"
                value={unit}
                onChange={(e) => setUnit(e.target.value as typeof unit)}
              >
                {["day", "week", "month"].map((value) => (
                  <option key={value}>{value}</option>
                ))}
              </select>
            </Label>
            <Label>
              Timezone
              <Input
                value={field?.type === "date" ? "UTC" : timezone}
                disabled={field?.type === "date"}
                onChange={(e) => setTimezone(e.target.value)}
              />
            </Label>
          </div>
          {result.isPending ? (
            <p role="status">Loading report…</p>
          ) : result.isError ? (
            <p role="alert">
              Could not load report. Check the timezone.{" "}
              <Button
                variant="outline"
                size="sm"
                onClick={() => void result.refetch()}
              >
                Retry report
              </Button>
            </p>
          ) : (
            <>
              <table className="w-full text-left text-12">
                <thead>
                  <tr>
                    <th>Bucket</th>
                    <th>Records</th>
                  </tr>
                </thead>
                <tbody>
                  {result.data?.points.map((point) => (
                    <tr key={String(point.key)}>
                      <td>{point.label}</td>
                      <td>{String(point.value ?? "Unavailable")}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
              <p className="text-11">
                Total: {String(result.data?.total ?? "Unavailable")}
                {result.data?.truncated
                  ? " · Additional buckets are combined as Other"
                  : ""}
              </p>
            </>
          )}
        </>
      ) : (
        <p className="text-12">
          Add a date or datetime field in Schema settings to create a dated
          report.
        </p>
      )}
    </section>
  );
}
