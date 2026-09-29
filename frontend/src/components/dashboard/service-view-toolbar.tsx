import { useRef, useState, type ReactNode, type Ref } from "react";
import { ArrowLeftRight, Layers, UserRound, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  ServiceFilterMultiselect,
  type ServiceFilterOption,
} from "./service-filter-multiselect";
import { ServiceOwnerAvatar } from "./service-owner-avatar";
import { ServiceSavedViews } from "./service-saved-views";
import {
  DataTableControls,
  DataTableSearch,
  DataTableFilterPopover,
  DataTableFilterChips,
} from "@/components/data-table/data-table-controls";
import {
  DEFAULT_SERVICE_FILTERS,
  serviceViewSchema,
} from "@/schemas/service-view";
import { connectionSource } from "@/lib/service-view";
import { cn } from "@/lib/utils";
import type { ServiceConnectionGroup } from "@/lib/service-groups";
import type { useServiceView } from "@/hooks/use-service-view";
import type { KeyInfo } from "@/types/keys";
import type {
  DataTableFilterField,
  DataTableFilterSelections,
} from "@/types/data-table";

type FilterKey = "source" | "state" | "service_type" | "show_auto_connected";
const SOURCE_LABELS = {
  personal: "Personal",
  org: "Organization",
  platform: "NyxID platform",
};

export function ServiceViewToolbar({
  view,
  keys,
  groups,
  children,
  ref,
  stuck = false,
}: {
  readonly view: ReturnType<typeof useServiceView>;
  readonly keys: readonly KeyInfo[];
  readonly groups: readonly ServiceConnectionGroup[];
  readonly children?: ReactNode;
  readonly ref?: Ref<HTMLDivElement>;
  readonly stuck?: boolean;
}) {
  const { filters, setFilters } = view;
  const inputRef = useRef<HTMLInputElement>(null);
  const [draft, setDraft] = useState<string | null>(null);
  const [open, setOpen] = useState(false);
  const [selectedKey, setSelectedKey] = useState<FilterKey>("source");
  const sources = new Set(keys.map(connectionSource));
  const organizations: ServiceFilterOption[] = [
    ...new Map(
      keys.flatMap((key) => {
        const source = key.credential_source;
        return source?.type === "org" ? [[source.org_id, source] as const] : [];
      }),
    ).values(),
  ]
    .sort((a, b) => a.org_name.localeCompare(b.org_name))
    .map((source) => ({
      id: source.org_id,
      label: source.org_name,
      icon: (
        <ServiceOwnerAvatar
          type="org"
          name={source.org_name}
          avatarUrl={source.avatar_url}
        />
      ),
    }));
  const services: ServiceFilterOption[] = groups.map((group) => ({
    id: group.id,
    label: group.name,
  }));
  const selections = [
    ...filters.organization_ids.map((id) => ({
      id,
      field: "organization_ids" as const,
      prefix: "Org",
      label:
        organizations.find((option) => option.id === id)?.label ??
        "Unavailable organization",
      icon: organizations.find((option) => option.id === id)?.icon,
    })),
    ...filters.service_group_ids.map((id) => ({
      id,
      field: "service_group_ids" as const,
      prefix: "Service",
      label:
        services.find((option) => option.id === id)?.label ??
        "Unavailable service",
      icon: undefined,
    })),
  ];
  const hasStandaloneFilters = selections.length > 0;
  const fields: readonly DataTableFilterField<FilterKey>[] = [
    {
      key: "source",
      label: "Source",
      value_type: "enum",
      operator: "is",
      options: (Object.keys(SOURCE_LABELS) as (keyof typeof SOURCE_LABELS)[])
        .filter((source) => sources.has(source))
        .map((value) => ({ value, label: SOURCE_LABELS[value] })),
    },
    {
      key: "state",
      label: "Service state",
      value_type: "enum",
      operator: "is",
      options: [
        { value: "enabled", label: "Enabled" },
        { value: "disabled", label: "Disabled" },
      ],
    },
    {
      key: "service_type",
      label: "Type",
      value_type: "enum",
      operator: "is",
      options: [
        { value: "http", label: "HTTP" },
        { value: "ssh", label: "SSH" },
      ],
    },
    {
      key: "show_auto_connected",
      label: "Auto-connected",
      value_type: "boolean",
      operator: "is",
      options: [
        { value: "false", label: "Hidden" },
        { value: "true", label: "Included" },
      ],
    },
  ];
  const values: DataTableFilterSelections<FilterKey> = {
    source: filters.source === "all" ? [] : [filters.source],
    state: filters.state === "all" ? [] : [filters.state],
    service_type: filters.service_type === "all" ? [] : [filters.service_type],
    show_auto_connected: filters.show_auto_connected ? [] : ["false"],
  };
  const applied = fields.flatMap((field) => {
    const selected = values[field.key] ?? [];
    return selected.length &&
      !(field.key === "source" && filters.source === "personal")
      ? [
          {
            field,
            values: selected,
            valueLabels: selected.map(
              (value) =>
                field.options.find((option) => option.value === value)?.label ??
                (field.key === "source"
                  ? SOURCE_LABELS[value as keyof typeof SOURCE_LABELS]
                  : value),
            ),
          },
        ]
      : [];
  });
  const editSearch = () => {
    setDraft(filters.search);
    inputRef.current?.focus();
  };
  const clear = () => {
    setDraft(null);
    setFilters({
      ...DEFAULT_SERVICE_FILTERS,
      source: filters.source === "personal" ? "personal" : "all",
    });
  };
  const sourceToggle = (
    <Button
      variant="outline"
      className="shrink-0 rounded-full"
      aria-label={`Service view: ${filters.source === "personal" ? "Personal" : "All services"}`}
      title={`Switch to ${filters.source === "personal" ? "All services" : "Personal"}`}
      onClick={() =>
        setFilters({
          ...filters,
          source: filters.source === "personal" ? "all" : "personal",
          organization_ids:
            filters.source === "personal" ? filters.organization_ids : [],
        })
      }
    >
      {filters.source === "personal" ? (
        <UserRound className="size-3.5" aria-hidden="true" />
      ) : (
        <Layers className="size-3.5" aria-hidden="true" />
      )}
      {filters.source === "personal" ? "Personal" : "All services"}
      <ArrowLeftRight
        className="ml-1 size-3 text-muted-foreground"
        aria-hidden="true"
      />
    </Button>
  );

  return (
    <div
      ref={ref}
      role="region"
      aria-label="Service filters"
      data-stuck={stuck}
      className="service-filter-toolbar pointer-events-none sticky top-0 z-20"
    >
      <div
        className={cn(
          "pointer-events-auto relative rounded-xl border border-border/60 bg-card transition-shadow duration-200 motion-reduce:transition-none",
          stuck
            ? "border-x-transparent border-t-transparent shadow-[0_4px_8px_-4px_rgb(0_0_0/0.4),0_16px_32px_-16px_rgb(0_0_0/0.6)] light:shadow-[0_4px_8px_-4px_rgb(0_0_0/0.12),0_16px_32px_-16px_rgb(0_0_0/0.2)]"
            : "shadow-sm",
        )}
      >
        {!stuck && (
          <div className="flex flex-wrap items-center justify-between gap-3 border-b border-border/50 px-4 py-3">
            <ServiceSavedViews
              view={view}
              onRestore={() => {
                setDraft(null);
                view.restoreDefault();
              }}
            />
            {sourceToggle}
          </div>
        )}
        <DataTableControls
          singleRow={stuck}
          className={stuck ? "border-b-0" : undefined}
          status={stuck ? sourceToggle : undefined}
          search={
            <>
              <div
                className={cn(
                  "flex gap-2",
                  stuck ? "shrink-0 flex-nowrap" : "w-full flex-wrap sm:w-auto",
                )}
              >
                <ServiceFilterMultiselect
                  label="Organization"
                  className={stuck ? "w-56 shrink-0 sm:w-56" : undefined}
                  plural="organizations"
                  options={organizations}
                  selected={filters.organization_ids}
                  onChange={(organization_ids) =>
                    setFilters({
                      ...filters,
                      organization_ids,
                      source:
                        organization_ids.length && filters.source === "personal"
                          ? "all"
                          : filters.source,
                    })
                  }
                />
                <ServiceFilterMultiselect
                  label="Service"
                  className={stuck ? "w-56 shrink-0 sm:w-56" : undefined}
                  plural="services"
                  options={services}
                  selected={filters.service_group_ids}
                  onChange={(service_group_ids) =>
                    setFilters({ ...filters, service_group_ids })
                  }
                />
              </div>
              <DataTableSearch
                fields={[]}
                selectedField={null}
                inputRef={inputRef}
                value={draft ?? filters.search}
                ariaLabel="Search services and connections"
                maxLength={200}
                onFieldChange={() => undefined}
                onValueChange={setDraft}
                onApply={() => {
                  setFilters({
                    ...filters,
                    search: (draft ?? filters.search).trim(),
                  });
                  setDraft(null);
                }}
                onCancel={() => setDraft("")}
              />
            </>
          }
          filter={
            <DataTableFilterPopover
              fields={fields}
              values={values}
              open={open}
              selectedKey={selectedKey}
              activeCount={applied.length + Number(Boolean(filters.search))}
              onOpenChange={setOpen}
              onSelectField={setSelectedKey}
              onApply={(selections) =>
                setFilters(
                  serviceViewSchema.parse({
                    ...filters,
                    source: selections.source?.[0] ?? "all",
                    state: selections.state?.[0] ?? "all",
                    service_type: selections.service_type?.[0] ?? "all",
                    show_auto_connected:
                      selections.show_auto_connected?.[0] !== "false",
                  }),
                )
              }
            />
          }
          chips={
            Boolean(selections.length || filters.search || applied.length) && (
              <div
                className="service-filter-pills"
                aria-label="Active service filters"
              >
                {selections.map((selection) => (
                  <div
                    key={`${selection.field}:${selection.id}`}
                    className="flex max-w-full items-center gap-1.5 rounded-md border border-border/80 bg-muted/25 pl-2 text-xs"
                  >
                    {selection.icon}
                    <span
                      className="min-w-0 truncate"
                      title={`${selection.prefix}: ${selection.label}`}
                    >
                      <span className="text-muted-foreground">
                        {selection.prefix}:
                      </span>{" "}
                      {selection.label}
                    </span>
                    <button
                      type="button"
                      aria-label={`Remove ${selection.prefix}: ${selection.label}`}
                      className="flex h-full w-11 shrink-0 items-center justify-center rounded text-muted-foreground hover:bg-accent hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring md:w-7"
                      onClick={() =>
                        setFilters({
                          ...filters,
                          [selection.field]: filters[selection.field].filter(
                            (id) => id !== selection.id,
                          ),
                        })
                      }
                    >
                      <X className="size-3" aria-hidden="true" />
                    </button>
                  </div>
                ))}
                <DataTableFilterChips
                  className="service-filter-extra-pills"
                  search={filters.search}
                  searchFields={[]}
                  searchFilters={[]}
                  filters={applied}
                  onEditSearch={editSearch}
                  onRemoveSearch={() => {
                    setDraft(null);
                    setFilters({ ...filters, search: "" });
                  }}
                  onEditSearchValue={editSearch}
                  onRemoveSearchValue={() => undefined}
                  onEdit={(key) => {
                    setSelectedKey(key);
                    setOpen(true);
                  }}
                  onRemove={(key) =>
                    setFilters({
                      ...filters,
                      [key]:
                        key === "source" ? "all" : DEFAULT_SERVICE_FILTERS[key],
                    })
                  }
                  onClear={clear}
                />
                {hasStandaloneFilters &&
                  !filters.search &&
                  applied.length === 0 && (
                    <Button
                      variant="ghost"
                      size="sm"
                      className="text-muted-foreground"
                      onClick={clear}
                    >
                      Clear filters
                    </Button>
                  )}
              </div>
            )
          }
        />

        {!stuck && (
          <div className="flex flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3">
            {children}
          </div>
        )}

        {!stuck && view.saveError && (
          <p role="alert" className="px-3 pb-2 text-xs text-destructive">
            {view.saveError}
          </p>
        )}
      </div>
    </div>
  );
}
