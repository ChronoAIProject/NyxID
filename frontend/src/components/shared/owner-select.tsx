import { useEffect, useState } from "react";
import { Command } from "cmdk";
import { Check, ChevronsUpDown, Search } from "lucide-react";
import { useOwnershipDestinations } from "@/hooks/use-ownership-transfers";
import type {
  OwnershipDestination,
  OwnershipResourceKind,
} from "@/types/ownership-transfers";
import { Button } from "@/components/ui/button";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";

export type SelectedOwner = Pick<
  OwnershipDestination,
  "id" | "display_name" | "email"
>;

export function OwnerSelect({
  id,
  kind,
  resourceId,
  ownerType,
  currentOwnerId,
  value,
  onChange,
  disabled,
}: {
  readonly id: string;
  readonly kind: OwnershipResourceKind;
  readonly resourceId: string;
  readonly ownerType: "person" | "org";
  readonly currentOwnerId: string;
  readonly value: SelectedOwner | null;
  readonly onChange: (owner: SelectedOwner) => void;
  readonly disabled?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState("");
  const [query, setQuery] = useState("");
  const [page, setPage] = useState(1);
  useEffect(() => {
    if (search.trim() === query) return;
    const timer = window.setTimeout(() => {
      setQuery(search.trim());
      setPage(1);
    }, 200);
    return () => window.clearTimeout(timer);
  }, [search, query]);
  const owners = useOwnershipDestinations(
    kind,
    resourceId,
    page,
    query,
    ownerType,
    open && !disabled,
  );
  const loading = search.trim() !== query || owners.isFetching;
  const candidates = loading || owners.error ? [] : (owners.data?.users ?? []);

  return (
    <Popover open={open && !disabled} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          id={id}
          type="button"
          variant="outline"
          role="combobox"
          aria-label="Destination owner"
          aria-expanded={open && !disabled}
          disabled={disabled}
          className="h-auto min-h-10 w-full justify-between gap-3 text-left font-normal"
        >
          {value ? (
            <span className="min-w-0">
              <span className="block truncate">
                {value.display_name || value.email}
              </span>
              {value.display_name && (
                <span className="block truncate text-xs text-muted-foreground">
                  {value.email}
                </span>
              )}
            </span>
          ) : (
            <span className="text-muted-foreground">
              Select destination owner
            </span>
          )}
          <ChevronsUpDown
            className="h-4 w-4 shrink-0 text-muted-foreground"
            aria-hidden="true"
          />
        </Button>
      </PopoverTrigger>
      <PopoverContent
        align="start"
        className="w-[var(--radix-popover-trigger-width)] p-0"
      >
        <Command label="Search owners" shouldFilter={false}>
          <div className="flex items-center gap-2 border-b px-3">
            <Search
              className="h-4 w-4 shrink-0 text-muted-foreground"
              aria-hidden="true"
            />
            <Command.Input
              value={search}
              onValueChange={setSearch}
              aria-label="Search owners"
              placeholder={
                ownerType === "person"
                  ? "Search people or enter exact email…"
                  : "Search organizations or enter ID…"
              }
              className="h-10 w-full bg-transparent text-sm outline-none placeholder:text-muted-foreground"
            />
          </div>
          <Command.List
            className="max-h-56 overflow-y-auto p-1"
            aria-busy={loading}
          >
            {loading && (
              <p role="status" className="p-3 text-sm text-muted-foreground">
                Searching…
              </p>
            )}
            {!loading && !owners.error && candidates.length === 0 && (
              <p className="p-3 text-sm text-muted-foreground">
                No matching owners.
              </p>
            )}
            {candidates.map((owner) => (
              <Command.Item
                key={owner.id}
                value={owner.id}
                disabled={!owner.is_active || owner.id === currentOwnerId}
                onSelect={() => {
                  onChange(owner);
                  setOpen(false);
                }}
                className="flex cursor-pointer items-center gap-2 rounded-md px-2 py-2 text-sm data-[disabled=true]:pointer-events-none data-[disabled=true]:opacity-50 data-[selected=true]:bg-accent data-[selected=true]:text-accent-foreground"
              >
                <span className="min-w-0 flex-1">
                  <span className="block truncate">
                    {owner.display_name || owner.email}
                    {!owner.is_active
                      ? " (inactive)"
                      : owner.id === currentOwnerId
                        ? " (current owner)"
                        : ""}
                  </span>
                  {owner.display_name && (
                    <span className="block truncate text-xs text-muted-foreground">
                      {owner.email}
                    </span>
                  )}
                </span>
                {owner.id === value?.id && (
                  <Check className="h-4 w-4 shrink-0" aria-hidden="true" />
                )}
              </Command.Item>
            ))}
          </Command.List>
          {owners.error && (
            <div className="p-3 text-sm">
              <p role="alert" className="text-destructive">
                Could not load owners.
              </p>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={() => void owners.refetch()}
              >
                Retry
              </Button>
            </div>
          )}
          {(page > 1 || (owners.data?.total ?? 0) > 20) && (
            <div className="flex items-center justify-between border-t p-1">
              <Button
                type="button"
                variant="ghost"
                size="sm"
                disabled={loading || page === 1}
                onClick={() => setPage((current) => current - 1)}
              >
                Previous owners
              </Button>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                disabled={loading || page * 20 >= (owners.data?.total ?? 0)}
                onClick={() => setPage((current) => current + 1)}
              >
                More owners
              </Button>
            </div>
          )}
        </Command>
      </PopoverContent>
    </Popover>
  );
}
