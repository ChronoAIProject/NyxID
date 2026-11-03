import { useState } from "react";
import { Check } from "lucide-react";
import {
  GroupedList,
  GroupedRow,
  GroupedInput,
} from "@/components/ui/grouped-list";
import { useNodes } from "@/hooks/use-nodes";
import { useSavedLogins } from "@/hooks/use-saved-logins";

export function MachineGrantPicker({
  kind,
  value,
  onChange,
  disabled,
  org,
}: {
  readonly org?: string;
  readonly kind: "machines" | "logins";
  readonly value: readonly string[];
  readonly onChange: (ids: string[]) => void;
  readonly disabled?: boolean;
}) {
  const [search, setSearch] = useState("");
  const nodes = useNodes();
  const logins = useSavedLogins("all");
  const rows =
    kind === "machines"
      ? (nodes.data ?? [])
          .filter(
            (node) =>
              (!org || node.owner?.id === org) &&
              node.machine &&
              (node.machine.shell ||
                node.machine.files ||
                node.machine.computer ||
                node.machine.browser),
          )
          .map((node) => ({ id: node.id, label: node.name }))
      : (logins.data ?? []).map((login) => ({
          id: login.id,
          label: login.label,
        }));
  const unknown = value
    .filter((id) => !rows.some((row) => row.id === id))
    .map((id) => ({ id, label: `${id} (unavailable)` }));
  const error = kind === "machines" ? nodes.error : logins.error;
  const allRows = [...rows, ...unknown];
  const filtered = allRows.filter((row) =>
    row.label.toLowerCase().includes(search.toLowerCase()),
  );
  return (
    <fieldset disabled={disabled} className="min-w-0">
      <legend className="sr-only">
        {kind === "machines" ? "Machines" : "Saved logins"}
      </legend>
      <GroupedList
        title={kind === "machines" ? "Machines" : "Saved logins"}
        footer={
          error ? (
            <span role="alert" className="text-destructive">
              {error.message}
            </span>
          ) : undefined
        }
      >
        {allRows.length > 8 ? (
          <GroupedRow>
            <GroupedInput
              aria-label={`Search ${kind === "machines" ? "machines" : "saved logins"}`}
              placeholder="Search"
              value={search}
              onChange={(event) => setSearch(event.target.value)}
            />
          </GroupedRow>
        ) : null}
        <div
          className={
            allRows.length > 12
              ? "assistant-scrollbar max-h-80 overflow-y-auto"
              : undefined
          }
        >
          {filtered.map((row) => (
            <GroupedRow key={row.id} asChild interactive>
              <button
                type="button"
                role="checkbox"
                aria-checked={value.includes(row.id)}
                disabled={disabled}
                onClick={() =>
                  onChange(
                    value.includes(row.id)
                      ? value.filter((id) => id !== row.id)
                      : [...value, row.id],
                  )
                }
              >
                <span className="min-w-0 flex-1 break-words">{row.label}</span>
                <Check
                  aria-hidden
                  className={
                    value.includes(row.id)
                      ? "size-[1.125em] shrink-0 text-primary"
                      : "invisible size-[1.125em] shrink-0"
                  }
                />
              </button>
            </GroupedRow>
          ))}
          {!filtered.length ? (
            <GroupedRow
              title={search ? "No matches" : "None available."}
              description={
                search
                  ? undefined
                  : kind === "machines"
                    ? "Pair a machine first, then grant it here."
                    : "Save a login first, then grant it here."
              }
            />
          ) : null}
        </div>
      </GroupedList>
    </fieldset>
  );
}
