import { useDeferredValue, useMemo, useState } from "react";
import { Search } from "lucide-react";
import { useAdminUsers } from "@/hooks/use-admin";
import { resolveServiceBillingMetric } from "@/lib/billing-units";
import type { AdminUser } from "@/types/admin";
import type { DownstreamService } from "@/types/api";
import { Badge } from "@/components/ui/badge";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";

export function UserPicker({
  selected,
  onChange,
}: {
  readonly selected: readonly string[];
  readonly onChange: (ids: string[]) => void;
}) {
  const [search, setSearch] = useState("");
  const deferredSearch = useDeferredValue(search.trim());
  const peopleQuery = useAdminUsers(
    1,
    100,
    deferredSearch || undefined,
    "person",
  );
  const organizationsQuery = useAdminUsers(
    1,
    100,
    deferredSearch || undefined,
    "org",
  );
  const users = useMemo(() => {
    const byId = new Map<string, AdminUser>();
    for (const user of [
      ...(peopleQuery.data?.users ?? []),
      ...(organizationsQuery.data?.users ?? []),
    ]) {
      if (user.is_active) byId.set(user.id, user);
    }
    return [...byId.values()];
  }, [organizationsQuery.data?.users, peopleQuery.data?.users]);
  const filtered = useMemo(() => {
    const needle = search.trim().toLowerCase();
    return needle
      ? users.filter((user) =>
          `${user.display_name ?? ""} ${user.email} ${user.slug ?? ""}`
            .toLowerCase()
            .includes(needle),
        )
      : users;
  }, [search, users]);
  const loading = peopleQuery.isFetching || organizationsQuery.isFetching;

  return (
    <div className="flex min-h-0 flex-col overflow-hidden rounded-lg border border-border">
      <div className="relative shrink-0 border-b border-border">
        <Search className="absolute left-3 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
        <Input
          value={search}
          onChange={(event) => setSearch(event.target.value)}
          placeholder="Search owners"
          className="border-0 pl-9 focus-visible:ring-0"
        />
      </div>
      <div className="min-h-0 max-h-48 flex-1 overflow-y-auto overscroll-contain p-1">
        {filtered.map((user) => {
          const checked = selected.includes(user.id);
          return (
            <label
              key={user.id}
              className="flex cursor-pointer items-center gap-3 rounded-md px-2 py-2 hover:bg-muted/50"
            >
              <Checkbox
                checked={checked}
                onCheckedChange={(value) =>
                  onChange(
                    value === true
                      ? [...selected, user.id]
                      : selected.filter((id) => id !== user.id),
                  )
                }
              />
              <span className="min-w-0 flex-1">
                <span className="block truncate text-[12px] font-medium">
                  {user.display_name || user.slug || user.email}
                </span>
                <span className="block truncate text-[11px] text-muted-foreground">
                  {user.slug ? `Organization / ${user.slug}` : user.email}
                </span>
              </span>
            </label>
          );
        })}
        {filtered.length === 0 && !loading ? (
          <p className="px-3 py-6 text-center text-[12px] text-muted-foreground">
            No owners found.
          </p>
        ) : null}
        {loading ? (
          <p className="px-3 py-3 text-center text-[11px] text-muted-foreground">
            Searching...
          </p>
        ) : null}
      </div>
      <div className="shrink-0 border-t border-border px-3 py-1.5 text-[11px] text-muted-foreground">
        {selected.length} selected
      </div>
    </div>
  );
}

export function ServicePicker({
  services,
  selected,
  onChange,
  multiple = false,
}: {
  readonly services: readonly DownstreamService[];
  readonly selected: readonly string[];
  readonly onChange: (ids: string[]) => void;
  readonly multiple?: boolean;
}) {
  const [search, setSearch] = useState("");
  const filtered = useMemo(() => {
    const needle = search.trim().toLowerCase();
    return services.filter(
      (service) =>
        service.is_active &&
        (!needle ||
          `${service.name} ${service.slug}`.toLowerCase().includes(needle)),
    );
  }, [search, services]);

  return (
    <div className="flex min-h-0 flex-col overflow-hidden rounded-lg border border-border">
      <div className="relative shrink-0 border-b border-border">
        <Search className="absolute left-3 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
        <Input
          value={search}
          onChange={(event) => setSearch(event.target.value)}
          placeholder="Search services"
          className="border-0 pl-9 focus-visible:ring-0"
        />
      </div>
      <div className="min-h-0 max-h-44 flex-1 overflow-y-auto overscroll-contain p-1">
        {filtered.map((service) => {
          const checked = selected.includes(service.id);
          const metric = resolveServiceBillingMetric(service);
          return (
            <label
              key={service.id}
              className="flex cursor-pointer items-center gap-3 rounded-md px-2 py-2 hover:bg-muted/50"
            >
              <Checkbox
                checked={checked}
                onCheckedChange={(value) =>
                  onChange(
                    value === true
                      ? multiple
                        ? [...selected, service.id]
                        : [service.id]
                      : selected.filter((id) => id !== service.id),
                  )
                }
              />
              <span className="min-w-0 flex-1">
                <span className="block truncate text-[12px] font-medium">
                  {service.name}
                </span>
                <span className="block truncate text-[11px] text-muted-foreground">
                  {service.slug}
                </span>
              </span>
              <Badge variant="secondary" className="shrink-0">
                {metric}
              </Badge>
            </label>
          );
        })}
        {filtered.length === 0 ? (
          <p className="px-3 py-6 text-center text-[12px] text-muted-foreground">
            No services found.
          </p>
        ) : null}
      </div>
    </div>
  );
}
