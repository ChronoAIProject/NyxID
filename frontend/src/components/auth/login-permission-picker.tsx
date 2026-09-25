import { useRef, useState } from "react";
import { Anchor as PopoverAnchor } from "@radix-ui/react-popover";
import { ChevronDown, Search, X } from "lucide-react";
import { Input } from "@/components/ui/input";
import { Checkbox } from "@/components/ui/checkbox";
import { Button } from "@/components/ui/button";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { ServiceIcon } from "@/components/service-icon";
import { NyxidIcon } from "@/components/brand/nyxid-icon";
import {
  normalizeRequested,
  type PermissionOption,
} from "@/lib/login-permissions";
import type { RequestedPermissions } from "@/schemas/login-request";
import {
  LoginConnectionChoices,
  type ConnectionSelection,
} from "./login-connection-choices";

export function PermissionIcon({ group }: { group: string }) {
  return group === "nyxid" ? (
    <NyxidIcon className="size-4 shrink-0" />
  ) : (
    <ServiceIcon slug={group} size="xs" />
  );
}

export function LoginPermissionPicker({
  options,
  value,
  initial,
  onChange,
  disabled,
  connections,
}: {
  options: PermissionOption[];
  value: RequestedPermissions;
  initial: RequestedPermissions;
  onChange: (value: RequestedPermissions) => void;
  disabled: boolean;
  connections?: ConnectionSelection;
}) {
  const [search, setSearch] = useState("");
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const searchInput = useRef<HTMLInputElement>(null);
  const selectedPanel = useRef<HTMLElement>(null);
  const title = useRef<HTMLHeadingElement>(null);
  const selected = normalizeRequested(value);
  const linkFilters = normalizeRequested(initial);
  const hasLinkFilters = Object.values(linkFilters).some(
    (items) => items.length > 0,
  );
  const groups = new Map<string, PermissionOption[]>();
  for (const option of options)
    groups.set(option.group, [...(groups.get(option.group) ?? []), option]);
  const total =
    selected.permissions.length +
    selected.services.length +
    selected.service_permissions.length;
  const chosen = (option: PermissionOption) =>
    selected[option.field].includes(option.value);
  function toggle(items: PermissionOption[], checked: boolean) {
    const next = structuredClone(selected);
    for (const item of items) {
      next[item.field] = next[item.field].filter((v) => v !== item.value);
      if (checked) next[item.field].push(item.value);
      if (item.field === "service_permissions")
        next.services = next.services.filter((v) => v !== item.group);
    }
    onChange(next);
  }
  function remove(items: PermissionOption[], button: HTMLButtonElement) {
    const buttons = [
      ...(selectedPanel.current?.querySelectorAll<HTMLButtonElement>(
        "button[data-permission-remove]",
      ) ?? []),
    ];
    const index = buttons.indexOf(button);
    toggle(items, false);
    requestAnimationFrame(() => {
      const remaining =
        selectedPanel.current?.querySelectorAll<HTMLButtonElement>(
          "button[data-permission-remove]",
        );
      (
        remaining?.[Math.min(index, remaining.length - 1)] ?? title.current
      )?.focus({ preventScroll: true });
    });
  }
  const visibleGroups = [...groups]
    .map(([id, items]) => ({
      id,
      items,
      visible: items.filter(
        (o) =>
          `${o.service} ${o.label} ${o.description} ${o.value}`
            .toLowerCase()
            .includes(search.toLowerCase()) ||
          !!connections?.inventory.connections.some(
            (c) =>
              (c.catalog_service_slug ?? c.slug) === id &&
              `${c.label} ${c.slug}`
                .toLowerCase()
                .includes(search.toLowerCase()),
          ),
      ),
    }))
    .filter((g) => g.visible.length);
  const selectedOptions = (
    Object.keys(selected) as (keyof RequestedPermissions)[]
  ).flatMap((field) =>
    selected[field].map(
      (v) =>
        options.find((o) => o.field === field && o.value === v) ?? {
          id: `${field}:${v}`,
          group: field === "permissions" ? "nyxid" : (v.split("::")[0] ?? v),
          service: field === "permissions" ? "NyxID" : (v.split("::")[0] ?? v),
          label: v.split("::")[1] ?? v,
          field,
          value: v,
          description: "Requested filter",
        },
    ),
  );
  const selectedGroups = new Map<string, PermissionOption[]>();
  for (const option of selectedOptions)
    selectedGroups.set(option.group, [
      ...(selectedGroups.get(option.group) ?? []),
      option,
    ]);
  return (
    <div className="space-y-3 text-[12px]">
      <div className="space-y-1">
        <div className="flex justify-between gap-2">
          <h3
            ref={title}
            tabIndex={-1}
            id="selected-permissions-title"
            className="font-semibold"
          >
            Permission filters
          </h3>
          <span role="status" className="text-[11px]">
            {total} selected
          </span>
        </div>
        <p className="text-[11px] text-muted-foreground">
          {connections
            ? "Find permissions and choose the connected accounts to include."
            : "Find keys that include these permissions. Matching keys may grant more access."}
        </p>
      </div>
      <Popover open={open && !disabled} onOpenChange={setOpen}>
        <PopoverAnchor asChild>
          <div
            ref={root}
            className="relative"
            onKeyDown={(event) => {
              if (event.key === "Escape") {
                event.preventDefault();
                searchInput.current?.focus({ preventScroll: true });
                setOpen(false);
              }
            }}
          >
            <label htmlFor="login-permission-search" className="sr-only">
              {connections
                ? "Search permissions & connections"
                : "Search permissions"}
            </label>
            <div className="relative">
              <Search className="pointer-events-none absolute left-3 top-2.5 size-3 text-muted-foreground" />
              <Input
                ref={searchInput}
                id="login-permission-search"
                value={search}
                disabled={disabled}
                className="pl-8 pr-10"
                placeholder={
                  connections
                    ? "Find a service, account, or permission…"
                    : "Search services or permissions…"
                }
                aria-expanded={open && !disabled}
                aria-controls="login-permission-options"
                aria-haspopup="dialog"
                onFocus={() => setOpen(true)}
                onClick={() => setOpen(true)}
                onChange={(e) => {
                  setSearch(e.target.value);
                  setOpen(true);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") e.preventDefault();
                  if (e.key === "ArrowDown") {
                    e.preventDefault();
                    setOpen(true);
                    requestAnimationFrame(() =>
                      menu.current
                        ?.querySelector<HTMLButtonElement>(
                          '[role="checkbox"]:not(:disabled)',
                        )
                        ?.focus({ preventScroll: true }),
                    );
                  }
                }}
              />
              <PopoverTrigger asChild>
                <button
                  type="button"
                  aria-label="Show permission options"
                  aria-controls="login-permission-options"
                  disabled={disabled}
                  className="absolute right-0 top-0 flex h-8 w-9 items-center justify-center"
                >
                  <ChevronDown className="size-3" />
                </button>
              </PopoverTrigger>
            </div>
          </div>
        </PopoverAnchor>
        <PopoverContent
          ref={menu}
          align="start"
          aria-label="Permission options"
          className="flex w-[var(--radix-popover-trigger-width)] max-h-[var(--radix-popover-content-available-height)] flex-col rounded-lg bg-card p-3 text-[12px]"
          id="login-permission-options"
          onOpenAutoFocus={(event) => event.preventDefault()}
          onCloseAutoFocus={(event) => event.preventDefault()}
          onInteractOutside={(event) => {
            if (root.current?.contains(event.target as Node))
              event.preventDefault();
          }}
          onEscapeKeyDown={() => {
            searchInput.current?.focus({ preventScroll: true });
          }}
        >
          <div className="mb-2 flex shrink-0 justify-between text-[11px] text-muted-foreground">
            <span>
              {connections
                ? "Permissions and connected accounts"
                : "Available permissions"}
            </span>
            <span>{total} selected</span>
          </div>
          <div
            className="min-h-0 max-h-72 overflow-y-auto overscroll-contain"
            aria-label="Available permissions"
          >
            {visibleGroups.map(({ id, items, visible }) => {
              const count = items.filter(chosen).length;
              return (
                <fieldset
                  key={id}
                  disabled={disabled}
                  aria-label={items[0]!.service}
                  className="mb-3 space-y-2"
                >
                  <legend className="mb-2 flex w-full items-center gap-2 font-medium">
                    <PermissionIcon group={id} />
                    {items[0]!.service}
                  </legend>
                  {connections && id !== "nyxid" && (
                    <LoginConnectionChoices
                      selection={connections}
                      requested={selected}
                      group={id}
                      disabled={disabled}
                    />
                  )}
                  {items[0]!.field !== "services" && (
                    <label className="flex items-center gap-2 py-1">
                      <Checkbox
                        aria-label={`${items[0]!.service}: All ${items.length} permissions`}
                        checked={
                          count === items.length
                            ? true
                            : count
                              ? "indeterminate"
                              : false
                        }
                        onCheckedChange={(v) => toggle(items, v === true)}
                      />
                      <span>Select all {items.length}</span>
                      {count > 0 && (
                        <span className="ml-auto text-muted-foreground">
                          {count}/{items.length}
                        </span>
                      )}
                    </label>
                  )}
                  {visible.map((option) => (
                    <label
                      key={option.id}
                      className="flex items-start gap-2 rounded-md py-1 pl-3 hover:bg-white/[0.03]"
                      title={[option.value, option.description]
                        .filter(Boolean)
                        .join(" · ")}
                    >
                      <Checkbox
                        aria-label={`${option.service}: ${option.label}`}
                        checked={chosen(option)}
                        onCheckedChange={(v) => toggle([option], v === true)}
                      />
                      <span className="min-w-0 break-words">
                        <span>{option.label}</span>
                        <span className="sr-only">{option.description}</span>
                      </span>
                    </label>
                  ))}
                </fieldset>
              );
            })}
            {!visibleGroups.length && <p>No matching service or permission.</p>}
          </div>
        </PopoverContent>
      </Popover>
      <section
        ref={selectedPanel}
        aria-labelledby="selected-permissions-title"
        className="space-y-2"
      >
        <div className="max-h-48 space-y-2 overflow-y-auto">
          {[...selectedGroups].map(([id, items]) => {
            const group = groups.get(id) ?? [];
            const all =
              group.length > 1 &&
              group.every(chosen) &&
              items.length === group.length;
            return (
              <div key={id} className="flex flex-wrap items-center gap-1.5">
                <span className="mr-1 flex items-center gap-1.5 text-[11px] text-muted-foreground">
                  <PermissionIcon group={id} />
                  {items[0]!.service}
                </span>
                <div className="contents">
                  {(all ? [items] : items.map((i) => [i])).map((pill) => (
                    <button
                      type="button"
                      key={pill[0]!.id}
                      disabled={disabled}
                      data-permission-remove
                      title={pill
                        .map((p) => `${p.service}: ${p.value}`)
                        .join(", ")}
                      aria-label={`Remove ${items[0]!.service}: ${all ? `All ${items.length} permissions` : pill[0]!.label}`}
                      className="flex max-w-full items-center gap-2 rounded-md border border-border bg-overlay px-2 py-1 text-[11px] hover:bg-overlay-strong focus-visible:outline-2 focus-visible:outline-primary"
                      onClick={(e) => remove(pill, e.currentTarget)}
                    >
                      <span className="break-all">
                        {all
                          ? `All ${items.length} permissions`
                          : pill[0]!.label}
                      </span>
                      <X className="size-3 shrink-0" />
                    </button>
                  ))}
                </div>
              </div>
            );
          })}
          {!total && (
            <p className="text-muted-foreground">
              {connections
                ? "Search above to choose permissions."
                : "No filters. Showing all eligible keys."}
            </p>
          )}
        </div>
        <div className="flex items-center justify-between gap-2">
          {hasLinkFilters ? (
            <Button
              variant="link"
              size="sm"
              className="h-auto px-0 py-1 text-[11px]"
              disabled={disabled}
              onClick={() => onChange(linkFilters)}
            >
              Reset to link filters
            </Button>
          ) : (
            <span />
          )}
          {total > 0 && (
            <Button
              variant="link"
              size="sm"
              disabled={disabled}
              className="h-auto px-0 py-1 text-[11px]"
              onClick={() =>
                onChange({
                  permissions: [],
                  services: [],
                  service_permissions: [],
                })
              }
            >
              Clear filters
            </Button>
          )}
        </div>
      </section>
    </div>
  );
}
