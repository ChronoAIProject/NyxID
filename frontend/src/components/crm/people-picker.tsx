import { createContext, useContext, useState, type ReactNode } from "react";
import {
  useCrmPeople,
  useCrmPerson,
  useCrmPeopleLabels,
} from "@/hooks/use-crm";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectTrigger,
  SelectContent,
  SelectItem,
  SelectValue,
} from "@/components/ui/select";
import { CrmOwnerContext } from "@/lib/crm/owner-context";
const LabelsContext = createContext<
  ReadonlyMap<string, { label: string; available: boolean }> | undefined
>(undefined);
export function CrmPeopleLabels({
  ids,
  ownerId,
  children,
}: {
  readonly ids: readonly string[];
  readonly ownerId?: string;
  readonly children: ReactNode;
}) {
  return ids.length ? (
    <LoadedCrmPeopleLabels ids={ids} ownerId={ownerId}>
      {children}
    </LoadedCrmPeopleLabels>
  ) : (
    <CrmOwnerContext value={ownerId}>
      <LabelsContext value={new Map()}>{children}</LabelsContext>
    </CrmOwnerContext>
  );
}
function LoadedCrmPeopleLabels({
  ids,
  ownerId,
  children,
}: {
  readonly ids: readonly string[];
  readonly ownerId?: string;
  readonly children: ReactNode;
}) {
  const labels = useCrmPeopleLabels(ids, ownerId);
  return (
    <CrmOwnerContext value={ownerId}>
      <LabelsContext
        value={
          labels.data
            ? new Map(labels.data.map((person) => [person.id, person]))
            : undefined
        }
      >
        {children}
      </LabelsContext>
    </CrmOwnerContext>
  );
}
export function CrmPersonName({ id }: { readonly id?: string }) {
  const labels = useContext(LabelsContext);
  return id
    ? (labels?.get(id)?.label ??
        (labels ? "Unavailable member" : "Loading member…"))
    : "—";
}
export function CrmPeoplePicker({
  value,
  ownerId,
  id,
  describedBy,
  disabled,
  onChange,
}: {
  readonly value: unknown;
  readonly ownerId?: string;
  readonly id?: string;
  readonly describedBy?: string;
  readonly disabled?: boolean;
  readonly onChange: (value: string | null) => void;
}) {
  const inherited = useContext(CrmOwnerContext);
  const owner = ownerId ?? inherited;
  const [cursors, setCursors] = useState<(string | undefined)[]>([undefined]);
  const [open, setOpen] = useState(false);
  const people = useCrmPeople(owner, cursors.at(-1), open);
  const selected = useCrmPerson(
    typeof value === "string" ? value : undefined,
    owner,
  );
  const choices = people.isError ? [] : (people.data?.people ?? []);
  return (
    <div className="space-y-2">
      <Select
        open={open}
        onOpenChange={setOpen}
        value={typeof value === "string" && value ? value : "__unset__"}
        disabled={disabled}
        onValueChange={(next) => onChange(next === "__unset__" ? null : next)}
      >
        <SelectTrigger
          id={id}
          aria-describedby={describedBy}
          aria-label={id ? undefined : "Member"}
        >
          <SelectValue>
            {typeof value === "string" && value
              ? selected.isError
                ? "Unavailable member"
                : (selected.data?.label ?? "Loading member…")
              : "Unassigned"}
          </SelectValue>
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="__unset__">Unassigned</SelectItem>
          {choices.map((person) => (
            <SelectItem
              key={person.id}
              value={person.id}
              disabled={!person.available}
            >
              {person.label}
            </SelectItem>
          ))}
          {open ? (
            <div className="flex gap-2">
              <Button
                type="button"
                size="sm"
                variant="outline"
                disabled={cursors.length < 2}
                onClick={() => setCursors((pages) => pages.slice(0, -1))}
              >
                Previous members
              </Button>
              <Button
                type="button"
                size="sm"
                variant="outline"
                disabled={!people.data?.next_cursor}
                onClick={() =>
                  setCursors((pages) => [
                    ...pages,
                    people.data?.next_cursor ?? undefined,
                  ])
                }
              >
                Next members
              </Button>
              {people.isError ? (
                <Button
                  type="button"
                  size="sm"
                  onClick={() => void people.refetch()}
                >
                  Retry members
                </Button>
              ) : null}
            </div>
          ) : null}
        </SelectContent>
      </Select>
    </div>
  );
}
