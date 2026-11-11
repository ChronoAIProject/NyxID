import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { Label } from "@/components/ui/label";
import { useDrawerSearchEntries } from "@/lib/drawer-search";
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
  footer,
}: {
  readonly org?: string;
  readonly footer?: React.ReactNode;
  readonly kind: "machines" | "logins";
  readonly value: readonly string[];
  readonly onChange: (ids: string[]) => void;
  readonly disabled?: boolean;
}) {
  useUiTranslation();
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
  useDrawerSearchEntries(allRows.map((row) => ({
    id: `${kind}-${row.id}`, title: row.label,
    breadcrumb: `Access › ${kind === "machines" ? "Machines" : "Saved logins"}`,
    keywords: value.includes(row.id) ? "Granted" : "Available", reveal: () => setSearch(""),
  })));
  const filtered = allRows.filter((row) =>
    row.label.toLowerCase().includes(search.toLowerCase()),
  );
  return (
    <fieldset disabled={disabled} className="min-w-0">
      <Label asChild><legend className="sr-only">
        {kind === "machines" ? translateUi("Machines") : translateUi("Saved logins")}
      </legend></Label>
      <GroupedList
        title={kind === "machines" ? translateUi("Machines") : translateUi("Saved logins")}
        footer={
          <>
            {footer}
            {error ? (
              <span role="alert" className="text-destructive">
                {error.message}
              </span>
            ) : null}
          </>
        }
      >
        {allRows.length > 8 ? (
          <GroupedRow>
            <GroupedInput
              aria-label={translateUi("Search {{value1}}", { value1: kind === "machines" ? "machines" : "saved logins" })}
              placeholder={translateUi("Search")}
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
            <GroupedRow data-drawer-search-target={`${kind}-${row.id}`} key={row.id} asChild interactive>
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
              title={search ? translateUi("No matches") : translateUi("None available.")}
              description={
                search
                  ? undefined
                  : kind === "machines"
                    ? translateUi("Pair a machine first, then grant it here.")
                    : translateUi("Save a login first, then grant it here.")
              }
            />
          ) : null}
        </div>
      </GroupedList>
    </fieldset>
  );
}
