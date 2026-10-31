import type { ReactNode } from "react";
import { Folder, Search, SlidersHorizontal, Tag } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

export interface FlagFilters {
  search: string;
  lifecycle: "active" | "sunset" | "all";
  tag: string;
  group: string;
  groupBy: "group" | "owner" | "none";
}

/**
 * Toolbar strip at the top of the flag table card, laid out like the audit
 * log's: search on the left, outline filter controls on the right, and a
 * result count underneath. Lifecycle tabs live above the card.
 */
export function FlagBrowserControls({
  filters,
  onChange,
  groups,
  tags,
  totalCount,
  sunsetCount,
  visibleCount,
}: {
  filters: FlagFilters;
  onChange: (patch: Partial<FlagFilters>) => void;
  groups: string[];
  tags: string[];
  totalCount: number;
  sunsetCount: number;
  visibleCount: number;
}) {
  const { search, lifecycle, tag, group: groupFilter, groupBy } = filters;
  const q = search.trim();
  return (
    <div className="space-y-2 border-b border-border/60 p-3">
      <div className="flex flex-col gap-2 lg:flex-row lg:items-center">
        <div className="relative min-w-0 flex-1">
          <Search
            className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-text-tertiary"
            aria-hidden
          />
          <Input
            value={search}
            onChange={(e) => onChange({ search: e.target.value })}
            placeholder="Search flags, groups, tags or owners"
            className="h-8 pl-9 text-12"
            aria-label="Search feature flags"
          />
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <FilterSelect
            icon={<Folder className="size-3.5 shrink-0" aria-hidden />}
            label="Filter by group"
            value={groupFilter}
            onChange={(value) => onChange({ group: value })}
          >
            <option value="">All groups</option>
            {groups.map((name) => (
              <option key={name} value={name}>
                {name}
              </option>
            ))}
          </FilterSelect>
          <FilterSelect
            icon={<Tag className="size-3.5 shrink-0" aria-hidden />}
            label="Filter by tag"
            value={tag}
            onChange={(value) => onChange({ tag: value })}
          >
            <option value="">All tags</option>
            {tags.map((name) => (
              <option key={name} value={name}>
                {name}
              </option>
            ))}
          </FilterSelect>
          <FilterSelect
            icon={
              <SlidersHorizontal className="size-3.5 shrink-0" aria-hidden />
            }
            label="Group flags by"
            value={groupBy}
            onChange={(value) =>
              onChange({ groupBy: value as FlagFilters["groupBy"] })
            }
          >
            <option value="group">By group</option>
            <option value="owner">By owner</option>
            <option value="none">No grouping</option>
          </FilterSelect>
        </div>
      </div>
      <div className="flex min-h-7 flex-wrap items-center justify-between gap-2 text-11 text-text-tertiary">
        <span>
          {visibleCount} of {totalCount} flags
          {lifecycle === "active" && sunsetCount > 0
            ? ` · ${sunsetCount} sunset ${sunsetCount === 1 ? "flag hidden" : "flags hidden"}`
            : ""}
        </span>
        {(q || tag || groupFilter) && (
          <Button
            variant="ghost"
            size="sm"
            className="h-7"
            onClick={() => onChange({ search: "", tag: "", group: "" })}
          >
            Clear filters
          </Button>
        )}
      </div>
    </div>
  );
}

/** A native select dressed as the outline "Filters" control. */
function FilterSelect({
  icon,
  label,
  value,
  onChange,
  children,
}: {
  icon: ReactNode;
  label: string;
  value: string;
  onChange: (value: string) => void;
  children: ReactNode;
}) {
  return (
    <label className="flex h-8 min-w-0 items-center gap-1.5 rounded-lg border border-hairline px-2.5 text-text-tertiary transition-colors hover:border-hairline-strong has-[select:focus-visible]:ring-2 has-[select:focus-visible]:ring-ring">
      {icon}
      <span className="sr-only">{label}</span>
      <select
        aria-label={label}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="h-full min-w-0 max-w-44 bg-transparent text-12 text-foreground focus-visible:outline-none"
      >
        {children}
      </select>
    </label>
  );
}
