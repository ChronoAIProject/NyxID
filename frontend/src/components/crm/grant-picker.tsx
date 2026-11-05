import { Label } from "@/components/ui/label";
import { useState } from "react";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { useAuthStore } from "@/stores/auth-store";
import { useCrmObjects } from "@/hooks/use-crm";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import type { z } from "zod";
import type { assistantCrmGrantSchema } from "@/schemas/assistant-nyxagent";
type Grant = z.infer<typeof assistantCrmGrantSchema>;
export function CrmGrantPicker({
  ownerId,
  value,
  onChange,
  disabled,
}: {
  readonly ownerId?: string;
  readonly value: readonly Grant[];
  readonly onChange: (value: Grant[]) => void;
  readonly disabled?: boolean;
}) {
  const [plannedKey, setPlannedKey] = useState("");
  const actor = useAuthStore((state) => state.user?.id);
  const owner = ownerId ?? actor;
  const enabled = useFeature(FEATURE_FLAG.CMS_POC_V1);
  const objects = useCrmObjects(owner, enabled && Boolean(owner));
  if (!owner || (!enabled && !value.length)) return null;
  return (
    <fieldset
      disabled={disabled || !enabled}
      className="space-y-3 rounded-lg border border-border p-4"
    >
      <Label asChild>
        <legend>CRM access</legend>
      </Label>
      <p className="text-12 text-text-secondary">
        Grant each object explicitly. Write and schema access also require read.
        Relationship matching and labels require read access to the target
        object. Guest turns cannot use CRM.
      </p>
      <div className="flex flex-wrap items-center gap-2">
        <Input
          className="w-48"
          aria-label="Planned object key"
          value={plannedKey}
          maxLength={48}
          onChange={(event) => setPlannedKey(event.target.value)}
          placeholder="new_object_key"
        />
        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={
            !/^[a-z][a-z0-9_]{0,47}$/.test(plannedKey) ||
            value.length >= 16 ||
            value.some((grant) => grant.object === plannedKey)
          }
          onClick={() => {
            onChange([
              ...value,
              {
                owner_id: owner,
                object: plannedKey,
                capabilities: ["read", "write", "schema"],
              },
            ]);
            setPlannedKey("");
          }}
        >
          Grant new object
        </Button>
      </div>
      <p className="text-12 text-text-secondary">
        A planned key grants permission to create that exact object and work
        with its records.
      </p>
      {objects.isError ? <p role="alert">Could not load CRM objects.</p> : null}
      {[
        ...(objects.data ?? []),
        ...value
          .filter(
            (grant) =>
              !objects.data?.some((object) => object.key === grant.object),
          )
          .map((grant) => ({
            key: grant.object,
            plural_label: `${grant.object} (planned)`,
          })),
      ].map((object) => {
        const grant = value.find(
          (entry) => entry.owner_id === owner && entry.object === object.key,
        );
        return (
          <div
            key={object.key}
            className="flex flex-wrap items-center gap-3 text-12"
          >
            <span className="min-w-28">{object.plural_label}</span>
            {(["read", "write", "schema"] as const).map((capability) => (
              <Label key={capability} className="flex items-center gap-1">
                <input
                  type="checkbox"
                  checked={grant?.capabilities.includes(capability) ?? false}
                  onChange={(event) => {
                    let caps = grant?.capabilities ?? [];
                    caps = event.target.checked
                      ? [...new Set([...caps, "read" as const, capability])]
                      : capability === "read"
                        ? []
                        : caps.filter((entry) => entry !== capability);
                    onChange([
                      ...value.filter(
                        (entry) =>
                          !(
                            entry.owner_id === owner &&
                            entry.object === object.key
                          ),
                      ),
                      ...(caps.length
                        ? [
                            {
                              owner_id: owner,
                              object: object.key,
                              capabilities: caps,
                            },
                          ]
                        : []),
                    ]);
                  }}
                />
                {capability === "schema"
                  ? "Edit schema"
                  : capability === "write"
                    ? "Write"
                    : "Read"}
              </Label>
            ))}
          </div>
        );
      })}
    </fieldset>
  );
}
