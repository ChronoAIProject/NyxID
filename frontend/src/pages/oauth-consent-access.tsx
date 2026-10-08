import { useState } from "react";
import { Search } from "lucide-react";
import { Input } from "@/components/ui/input";
import {
  rowLevel,
  type AccessLevel,
  type AccessServiceRow,
} from "@/lib/agent-grant";
import { cn } from "@/lib/utils";

const LEVEL_OPTIONS: readonly { value: AccessLevel; label: string }[] = [
  { value: "off", label: "Off" },
  { value: "read", label: "Read" },
  { value: "write", label: "Read & write" },
];

function LevelControl({
  label,
  value,
  onChange,
  offDisabled = false,
}: {
  readonly label: string;
  readonly value: AccessLevel;
  readonly onChange: (level: AccessLevel) => void;
  readonly offDisabled?: boolean;
}) {
  return (
    <div
      role="radiogroup"
      aria-label={label}
      className="flex shrink-0 gap-0.5 rounded-lg border border-hairline p-0.5"
    >
      {LEVEL_OPTIONS.map((option) => {
        const disabled = offDisabled && option.value === "off";
        return (
          <button
            key={option.value}
            type="button"
            role="radio"
            aria-checked={value === option.value}
            disabled={disabled}
            onClick={() => onChange(option.value)}
            className={cn(
              "h-6 whitespace-nowrap rounded-md px-2 text-11 transition-colors",
              value === option.value
                ? option.value === "write"
                  ? "bg-warning/15 font-medium text-foreground"
                  : "bg-overlay-strong font-medium text-foreground"
                : "text-muted-foreground hover:text-foreground",
              disabled &&
                "cursor-not-allowed opacity-40 hover:text-muted-foreground",
            )}
          >
            {option.label}
          </button>
        );
      })}
    </div>
  );
}

export interface ServiceAccessTableProps {
  readonly clientName: string;
  readonly rows: readonly AccessServiceRow[];
  readonly loading: boolean;
  readonly defaultLevel: AccessLevel;
  readonly onDefaultLevelChange: (level: AccessLevel) => void;
  readonly overrides: Readonly<Record<string, AccessLevel>>;
  readonly onLevelChange: (id: string, level: AccessLevel) => void;
  readonly className?: string;
}

/**
 * Service access for agent apps (Claude, Codex and other MCP clients) in one
 * table: every service is readable by default, and writing is only ever
 * enabled by an explicit choice.
 */
export function ServiceAccessTable({
  clientName,
  rows,
  loading,
  defaultLevel,
  onDefaultLevelChange,
  overrides,
  onLevelChange,
  className,
}: ServiceAccessTableProps) {
  const [query, setQuery] = useState("");
  const needle = query.trim().toLowerCase();
  const visibleRows = needle
    ? rows.filter((row) =>
        [row.name, row.secondary, row.orgName ?? ""].some((text) =>
          text.toLowerCase().includes(needle),
        ),
      )
    : rows;
  return (
    <section
      aria-labelledby="oauth-service-access"
      className={cn("border-t border-border py-5", className)}
    >
      <div className="flex items-baseline justify-between gap-3">
        <h2
          id="oauth-service-access"
          className="text-15 font-semibold text-foreground"
        >
          Service access
        </h2>
        <code className="text-11 text-muted-foreground">proxy</code>
      </div>
      <p className="mt-1 text-12 leading-relaxed text-muted-foreground">
        Read lets {clientName} look things up. Write also lets it create, update
        or delete, and you are still asked before risky actions.
      </p>

      {rows.length > 0 && (
        <div className="relative mt-3">
          <Search
            className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-text-tertiary"
            aria-hidden="true"
          />
          <Input
            type="search"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder={`Search ${rows.length} services`}
            aria-label="Search services"
            className="h-8 pl-8 text-12"
          />
        </div>
      )}

      <div className="mt-2 overflow-hidden rounded-lg border border-border/60">
        <div className="flex items-center justify-between gap-3 bg-overlay px-3 py-2">
          <div className="min-w-0">
            <p className="text-13 font-medium text-foreground">All services</p>
            <p className="text-11 text-text-tertiary">
              Also applies to services you connect later
            </p>
          </div>
          <LevelControl
            label="All services"
            value={defaultLevel}
            onChange={onDefaultLevelChange}
          />
        </div>
        <div className="max-h-72 divide-y divide-border/60 overflow-y-auto border-t border-border/60 lg:max-h-[26rem]">
          {loading ? (
            <p className="px-3 py-2.5 text-12 text-muted-foreground">
              Loading services...
            </p>
          ) : rows.length === 0 ? (
            <p className="px-3 py-2.5 text-12 text-muted-foreground">
              No services connected yet. New ones follow the setting above.
            </p>
          ) : visibleRows.length === 0 ? (
            <p className="px-3 py-2.5 text-12 text-muted-foreground">
              No services match &ldquo;{query.trim()}&rdquo;.
            </p>
          ) : (
            visibleRows.map((row) => (
              <div
                key={row.id}
                className="flex items-center justify-between gap-3 px-3 py-2"
              >
                <div className="min-w-0">
                  <p
                    className="truncate text-13 text-foreground"
                    title={row.name}
                  >
                    {row.name}
                  </p>
                  {(row.orgName || row.requiredByApp || row.secondary) && (
                    <p className="truncate text-11 text-text-tertiary">
                      {[
                        row.orgName && `Org · ${row.orgName}`,
                        row.requiredByApp ? "Requested by app" : row.secondary,
                      ]
                        .filter(Boolean)
                        .join(" · ")}
                    </p>
                  )}
                </div>
                <LevelControl
                  label={row.name}
                  value={rowLevel(row, defaultLevel, overrides)}
                  onChange={(level) => onLevelChange(row.id, level)}
                  offDisabled={row.requiredByApp}
                />
              </div>
            ))
          )}
        </div>
      </div>

      <p className="mt-2 text-11 leading-relaxed text-text-tertiary">
        {clientName} can see the names of all your connected services so it can
        tell you what to share. Credentials never leave NyxID.
      </p>
    </section>
  );
}
