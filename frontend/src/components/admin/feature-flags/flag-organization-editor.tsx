import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { useState } from "react";
import { Sunset } from "lucide-react";
import { toast } from "sonner";
import { useUpdateAdminFeatureFlagMetadata } from "@/hooks/use-admin-feature-flags";
import { useChangeReview } from "@/components/shared/change-review-dialog";
import { StaleFormNotice } from "@/components/shared/stale-form-notice";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import {
  changedFields,
  describeChanges,
  hasFieldConflicts,
  sameValue,
} from "@/lib/form-changes";
import {
  featureFlagTagsSchema,
  MAX_FEATURE_FLAG_GROUP_LENGTH,
} from "@/schemas/admin-feature-flags";

export interface FlagOrganization {
  group: string | null;
  tags: string[];
  sunset: boolean;
}

export function FlagOrganizationEditor({
  flagKey,
  current,
  canWrite,
  groups,
}: {
  flagKey: string;
  current: FlagOrganization;
  canWrite: boolean;
  groups: string[];
}) {
  useUiTranslation();
  const update = useUpdateAdminFeatureFlagMetadata();
  const [baseline, setBaseline] = useState(current);
  const [observed, setObserved] = useState(current);
  const [group, setGroup] = useState(current.group ?? "");
  const [tagText, setTagText] = useState(current.tags.join(", "));
  const [sunset, setSunset] = useState(current.sunset);
  const [hydrationPending, setHydrationPending] = useState(false);
  const tags = [
    ...new Set(
      tagText
        .split(",")
        .map((tag) => tag.trim().toLowerCase())
        .filter(Boolean),
    ),
  ].sort();
  const validation = featureFlagTagsSchema.safeParse(tags);
  const next = { group: group.trim() || null, tags, sunset };
  const patch = changedFields(baseline, next);
  const stale = hasFieldConflicts(baseline, current, patch);
  const dirty = Object.keys(patch).length > 0;

  function hydrate(values: FlagOrganization) {
    setBaseline(values);
    setGroup(values.group ?? "");
    setTagText(values.tags.join(", "));
    setSunset(values.sunset);
    setHydrationPending(false);
  }
  const review = useChangeReview<{
    body: Partial<FlagOrganization> & { expected_tags?: string[] };
    before: FlagOrganization;
  }>(
    async ({ body }) => {
      const saved = await update.mutateAsync({ flagKey, body });
      hydrate({
        group: saved.group ?? null,
        tags: saved.tags ?? [],
        sunset: saved.sunset ?? false,
      });
      toast.success(translateUi("Flag organization saved"));
    },
    (pending) =>
      hasFieldConflicts(
        pending.before,
        current,
        Object.fromEntries(
          Object.entries(pending.body).filter(
            ([key]) => key !== "expected_tags",
          ),
        ),
      ),
    flagKey,
  );
  const sourceChanged = !sameValue(observed, current);
  if (sourceChanged) setObserved(current);
  const pristine =
    group === (baseline.group ?? "") &&
    tagText === baseline.tags.join(", ") &&
    sunset === baseline.sunset;
  if (
    (sourceChanged || hydrationPending) &&
    pristine &&
    !review.saving &&
    !update.isPending
  )
    hydrate(current);
  else if (sourceChanged) setHydrationPending(true);

  if (!canWrite)
    return (
      <p className="py-1.5 text-14 text-muted-foreground">
        {current.group ?? translateUi("Grouped by namespace")} ·{" "}
        {current.tags.length ? current.tags.join(", ") : translateUi("No tags")} ·{" "}
        {current.sunset ? translateUi("Sunset") : translateUi("Active")}
      </p>
    );

  return (
    <div className="space-y-4">
      {review.dialog}
      {stale && (
        <StaleFormNotice
          onReload={() => {
            hydrate(current);
            review.cancel();
          }}
        />
      )}
      <div className="grid gap-4 sm:grid-cols-2">
        <div className="space-y-1">
          <Label htmlFor={`flag-group-${flagKey}`}>{translateUi("Group")}</Label>
          <Input
            id={`flag-group-${flagKey}`}
            value={group}
            onChange={(e) => setGroup(e.target.value)}
            maxLength={MAX_FEATURE_FLAG_GROUP_LENGTH}
            placeholder={translateUi("Use flag namespace")}
            list={`flag-groups-${flagKey}`}
            disabled={review.saving || update.isPending}
          />
          <datalist id={`flag-groups-${flagKey}`}>
            {groups.map((name) => (
              <option key={name} value={name} />
            ))}
          </datalist>
          <p className="text-13 text-muted-foreground">{translateUi("Keep related flags together. Leave blank to use the namespace.")}</p>
        </div>
        <div className="space-y-1">
          <Label htmlFor={`flag-tags-${flagKey}`}>{translateUi("Tags")}</Label>
          <Input
            id={`flag-tags-${flagKey}`}
            value={tagText}
            onChange={(e) => setTagText(e.target.value)}
            placeholder={translateUi("voice, rollout, infrastructure")}
            disabled={review.saving || update.isPending}
            aria-invalid={!validation.success}
            aria-describedby={`flag-tags-hint-${flagKey}`}
          />
          <p
            id={`flag-tags-hint-${flagKey}`}
            className={`text-13 ${validation.success ? "text-muted-foreground" : "text-destructive"}`}
            role={validation.success ? undefined : "alert"}
          >
            {validation.success
              ? translateUi("Comma-separated. Up to 12 tags; letters, digits, hyphens and underscores.")
              : validation.error.issues[0]?.message}
          </p>
        </div>
      </div>
      <div className="flex items-start gap-3">
        <Sunset className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden />
        <div className="flex-1 space-y-1">
          <Label htmlFor={`flag-sunset-${flagKey}`}>{translateUi("Mark for sunset")}</Label>
          <p className="text-14 text-muted-foreground">{translateUi("Hide this flag from the default view while code cleanup is pending. Its default and rollout overrides continue to apply. Turn this off to restore it to Active.")}</p>
        </div>
        <Switch
          id={`flag-sunset-${flagKey}`}
          className="border-border data-[state=unchecked]:bg-secondary"
          checked={sunset}
          onCheckedChange={setSunset}
          disabled={review.saving || update.isPending}
        />
      </div>
      <div className="flex justify-end">
        <Button
          size="sm"
          variant="primary"
          disabled={
            !dirty ||
            stale ||
            !validation.success ||
            review.saving ||
            update.isPending
          }
          onClick={() =>
            review.review(
              {
                body: {
                  ...patch,
                  ...(patch.tags ? { expected_tags: baseline.tags } : {}),
                },
                before: baseline,
              },
              describeChanges(baseline, patch, {
                labels: { sunset: "Marked for sunset" },
              }),
            )
          }
        >{translateUi("Save organization")}</Button>
      </div>
    </div>
  );
}
