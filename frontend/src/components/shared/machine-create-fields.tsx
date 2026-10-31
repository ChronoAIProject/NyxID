import { Checkbox } from "@/components/ui/checkbox";
export type MachineCapability = "shell" | "files" | "browser" | "computer";
/** Same machine ceiling controls in both setup adapters; these do not grant Agent tools. */
export function MachineCapabilityChoices({
  value,
  onChange,
  disabled,
}: {
  readonly value: MachineCapability[];
  readonly onChange: (value: MachineCapability[]) => void;
  readonly disabled: boolean;
}) {
  return (
    <fieldset disabled={disabled} className="space-y-3">
      <legend className="mb-2 text-12 font-medium">
        Enable on this machine
      </legend>
      {(["shell", "files", "browser", "computer"] as const).map(
        (capability) => (
          <label key={capability} className="flex items-center gap-2 text-12">
            <Checkbox
              aria-label={
                {
                  shell: "Commands",
                  files: "Files",
                  browser: "Browser",
                  computer: "Computer",
                }[capability]
              }
              checked={value.includes(capability)}
              onCheckedChange={(checked) => {
                const next = checked
                  ? [...value, capability]
                  : value.filter(
                      (item) =>
                        item !== capability &&
                        !(capability === "browser" && item === "computer"),
                    );
                onChange([
                  ...new Set(
                    checked && capability === "computer"
                      ? [...next, "browser" as const]
                      : next,
                  ),
                ]);
              }}
            />
            {
              {
                shell: "Commands",
                files: "Files",
                browser: "Browser",
                computer: "Computer",
              }[capability]
            }
          </label>
        ),
      )}
    </fieldset>
  );
}
