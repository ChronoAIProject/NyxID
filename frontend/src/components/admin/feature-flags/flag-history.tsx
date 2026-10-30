import { useState } from "react";
import { History } from "lucide-react";
import { useFeatureFlagHistory } from "@/hooks/use-admin-feature-flags";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import type { FeatureFlagHistoryEntry } from "@/schemas/admin-feature-flags";

function valueLabel(value: unknown): string {
  if (value === null || value === undefined) return "Inherit";
  if (typeof value === "boolean") return value ? "Enabled" : "Disabled";
  return typeof value === "object" ? JSON.stringify(value) : String(value);
}

function changes(entry: FeatureFlagHistoryEntry): string[] {
  const { details, event_type: event } = entry;
  if (event === "admin_feature_flag_created")
    return [
      `Default: ${valueLabel(details.default_enabled)}`,
      ...(details.description ? [String(details.description)] : []),
    ];
  if (event === "admin_feature_flag_metadata_set") {
    if (
      details.before &&
      details.after &&
      typeof details.before === "object" &&
      typeof details.after === "object"
    ) {
      const before = details.before as Record<string, unknown>;
      return Object.entries(details.after)
        .filter(
          ([key, value]) =>
            JSON.stringify(before[key]) !== JSON.stringify(value),
        )
        .map(([key, value]) => {
          const label =
            key === "sunset" ? "Status" : key[0]!.toUpperCase() + key.slice(1);
          const display = (v: unknown) =>
            key === "sunset"
              ? v
                ? "Sunset"
                : "Active"
              : v == null
                ? "Not set"
                : Array.isArray(v)
                  ? v.join(", ") || "No tags"
                  : String(v);
          return `${label}: ${display(before[key])} → ${display(value)}`;
        });
    }
    const legacyValues = ["description", "owner"]
      .filter((key) => Object.hasOwn(details, key))
      .map(
        (key) =>
          `${key === "owner" ? "Owner" : "Description"}: ${details[key] ?? "Not set"}`,
      );
    return legacyValues.length
      ? legacyValues
      : ["Metadata updated (older entries may not include changed values)."];
  }
  const scope =
    details.target_kind === "global"
      ? "Global · all users"
      : `${details.target_kind === "org" ? "Organization" : "User"}: ${entry.target_name ?? details.org_user_id ?? details.target_key ?? "unknown"}`;
  const after = event.endsWith("cleared")
    ? "Inherit (override removed)"
    : valueLabel(details.enabled);
  return [
    `${scope} · ${Object.hasOwn(details, "before") ? `${valueLabel(details.before)} → ` : ""}${after}`,
  ];
}

export function FlagHistory({
  flagKey,
  enabled,
}: {
  flagKey: string;
  enabled: boolean;
}) {
  const [cursors, setCursors] = useState<(string | null)[]>([null]);
  const history = useFeatureFlagHistory(
    flagKey,
    cursors.at(-1) ?? null,
    enabled,
  );
  return (
    <div className="space-y-3">
      <p className="text-12 text-muted-foreground">
        Who changed this flag, what changed, and when. Code-defined flags
        originate in source control; this history includes recorded admin
        changes.
      </p>
      {history.isLoading && <Skeleton className="h-24" />}
      {history.error && (
        <div role="alert" className="flex items-center gap-3 text-12">
          Unable to load flag history.
          <Button size="sm" onClick={() => void history.refetch()}>
            Retry history
          </Button>
        </div>
      )}
      {!history.isLoading &&
        !history.error &&
        !history.data?.entries.length && (
          <p className="py-6 text-center text-12 text-muted-foreground">
            No recorded changes for this flag.
          </p>
        )}
      {history.data && (
        <p className="text-12">
          {history.data.origin.created_by ? (
            <>
              Created by{" "}
              <span className="font-medium">
                {history.data.origin.creator_name ??
                  history.data.origin.created_by}
              </span>
              {history.data.origin.created_at
                ? ` on ${new Date(history.data.origin.created_at).toLocaleString()}`
                : ""}
              .
            </>
          ) : (
            "Declared in code. Creation and default changes are tracked in source control."
          )}
        </p>
      )}
      {history.data?.legacy_ambiguous_omitted && (
        <p className="text-11 text-muted-foreground">
          Some older changes may have been omitted because this flag key was
          reused and their identity cannot be verified.
        </p>
      )}
      <ol className="divide-y divide-border/60">
        {history.data?.entries.map((entry) => (
          <li key={entry.id} className="flex gap-3 py-3">
            <History
              className="mt-0.5 size-3.5 shrink-0 text-muted-foreground"
              aria-hidden
            />
            <div className="min-w-0 flex-1 space-y-1">
              <div className="flex flex-wrap items-center justify-between gap-2 text-12">
                <span>
                  <span
                    className="font-medium"
                    title={entry.actor_id ?? undefined}
                  >
                    {entry.actor_name ??
                      entry.actor_email ??
                      entry.actor_id ??
                      "System"}
                  </span>{" "}
                  <span className="text-muted-foreground">
                    {entry.event_type.endsWith("created")
                      ? "created this flag"
                      : entry.event_type.includes("metadata")
                        ? "updated flag details"
                        : entry.event_type.endsWith("cleared")
                          ? "removed an override"
                          : "changed rollout"}
                  </span>
                </span>
                <time
                  dateTime={entry.created_at}
                  title={new Date(entry.created_at).toISOString()}
                  className="text-11 text-muted-foreground"
                >
                  {new Date(entry.created_at).toLocaleString()}
                </time>
              </div>
              {entry.legacy && (
                <p className="text-11 text-muted-foreground">
                  Legacy record · flag identity inferred from available records
                </p>
              )}
              {changes(entry).map((change, i) => (
                <p
                  key={i}
                  className="break-words text-12 text-muted-foreground"
                >
                  {change}
                </p>
              ))}
            </div>
          </li>
        ))}
      </ol>
      {(cursors.length > 1 || history.data?.next_cursor) && (
        <div className="flex items-center justify-end gap-3 text-12">
          <Button
            size="sm"
            disabled={cursors.length === 1 || history.isFetching}
            onClick={() => setCursors((items) => items.slice(0, -1))}
          >
            Newer
          </Button>
          <span>Page {cursors.length}</span>
          <Button
            size="sm"
            disabled={!history.data?.next_cursor || history.isFetching}
            onClick={() =>
              setCursors((items) => [...items, history.data!.next_cursor!])
            }
          >
            Older
          </Button>
        </div>
      )}
    </div>
  );
}
