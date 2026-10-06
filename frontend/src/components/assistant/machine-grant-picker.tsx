import { Checkbox } from "@/components/ui/checkbox";
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
                node.machine.computer || node.machine.browser),
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
  return (
    <fieldset disabled={disabled} className="space-y-2">
      <legend className="mb-2 text-12 font-medium">
        {kind === "machines" ? "Machines" : "Saved logins"}
      </legend>
      {error ? (
        <p role="alert" className="text-12 text-destructive">
          {error.message}
        </p>
      ) : null}
      {[...rows, ...unknown].map((row) => (
        <label key={row.id} className="flex items-center gap-2 text-12">
          <Checkbox
            checked={value.includes(row.id)}
            onCheckedChange={(checked) =>
              onChange(
                checked
                  ? [...value, row.id]
                  : value.filter((id) => id !== row.id),
              )
            }
          />
          {row.label}
        </label>
      ))}
      {!rows.length && !unknown.length ? (
        <p className="text-12 text-muted-foreground">
          None available. The agent can ask NyxBot for access later.
        </p>
      ) : null}
    </fieldset>
  );
}
