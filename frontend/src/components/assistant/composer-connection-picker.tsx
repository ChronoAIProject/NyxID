import { useState } from "react";
import { Check, Link2, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { groupServiceConnections } from "@/lib/service-groups";
import { connectionOwnerLabel } from "@/lib/assistant/connection-context";
import type { KeyInfo } from "@/types/keys";

export function ComposerConnectionPicker({
  open,
  onOpenChange,
  connections,
  value,
  onChange,
  loading,
  error,
  onRetry,
  disabled,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly connections: readonly KeyInfo[];
  readonly value: string;
  readonly onChange: (id: string) => void;
  readonly loading: boolean;
  readonly error: boolean;
  readonly onRetry: () => void;
  readonly disabled: boolean;
}) {
  const [search, setSearch] = useState("");
  const query = search.trim().toLocaleLowerCase();
  const groups = groupServiceConnections(connections)
    .map((group) => ({
      ...group,
      connections: group.connections.filter((key) =>
        `${group.name} ${key.label} ${key.slug} ${connectionOwnerLabel(key)}`
          .toLocaleLowerCase()
          .includes(query),
      ),
    }))
    .filter((group) => group.connections.length);

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        setSearch("");
        onOpenChange(next);
      }}
    >
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Connection for this message</DialogTitle>
          <DialogDescription>
            Choose the account to use for this message. Manage your saved
            connection order in Agent Details.
          </DialogDescription>
        </DialogHeader>
        <Input
          aria-label="Search connections"
          placeholder="Search services or accounts…"
          value={search}
          onChange={(event) => setSearch(event.target.value)}
        />
        <div className="assistant-scrollbar max-h-[50vh] space-y-4 overflow-y-auto">
          {loading && (
            <p role="status" className="text-12 text-muted-foreground">
              Loading connections…
            </p>
          )}
          {error ? (
            <div role="alert" className="space-y-2 text-12">
              <p>Could not load connections.</p>
              <Button type="button" onClick={onRetry}>
                Retry
              </Button>
            </div>
          ) : (
            groups.map((group) => (
              <section
                key={group.id}
                aria-label={group.name}
                className="space-y-1"
              >
                <h3 className="text-12 font-semibold">{group.name}</h3>
                {group.connections.map((key) => (
                  <button
                    key={key.id}
                    type="button"
                    disabled={disabled}
                    aria-pressed={value === key.id}
                    className="flex w-full items-center gap-2 rounded-lg px-2 py-2 text-left text-12 hover:bg-overlay focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50"
                    onClick={() => {
                      onChange(key.id);
                      onOpenChange(false);
                      setSearch("");
                    }}
                  >
                    <span className="min-w-0 flex-1 break-words">
                      <span className="block font-medium">{key.label}</span>
                      <span className="text-11 text-muted-foreground">
                        {connectionOwnerLabel(key)} · {key.slug}
                      </span>
                    </span>
                    {value === key.id && (
                      <Check aria-hidden className="size-4 shrink-0" />
                    )}
                  </button>
                ))}
              </section>
            ))
          )}
          {!loading && !error && !groups.length && (
            <p className="text-12 text-muted-foreground">
              {query
                ? "No matching connections."
                : "No enabled connections are available."}
            </p>
          )}
        </div>
        <p className="text-11 text-muted-foreground">
          Organization connections still require approval in Auto and Ask every
          time.
        </p>
      </DialogContent>
    </Dialog>
  );
}

export function ComposerConnectionChip({
  connection,
  unavailable,
  disabled,
  onRemove,
}: {
  readonly connection: KeyInfo;
  readonly unavailable: boolean;
  readonly disabled: boolean;
  readonly onRemove: () => void;
}) {
  return (
    <div
      role="group"
      aria-label="Connection for this message"
      className="mb-2 ml-[30px] flex min-w-0 items-center gap-2 rounded-lg border border-border px-2 py-1 text-12"
    >
      <Link2 aria-hidden className="size-3.5 shrink-0 text-muted-foreground" />
      <span className="min-w-0 flex-1 break-words">
        <span className="font-medium">
          {connection.catalog_service_name ?? connection.label}
        </span>
        {" · "}
        {connection.label}
        {" · "}
        {connectionOwnerLabel(connection)}
        <span
          className={unavailable ? "text-destructive" : "text-muted-foreground"}
        >
          {unavailable
            ? " · Unavailable — choose another connection or remove"
            : " · This message"}
        </span>
      </span>
      <Button
        type="button"
        variant="ghost"
        size="icon"
        className="size-7 shrink-0"
        aria-label="Remove connection for this message"
        disabled={disabled}
        onClick={onRemove}
      >
        <X aria-hidden className="size-3.5" />
      </Button>
    </div>
  );
}
