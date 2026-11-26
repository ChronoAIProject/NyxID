import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { api } from "@/lib/api-client";
import { useUiTranslation } from "@/i18n/use-ui-translation";
import {
  GUEST_SKILL_AUDIENCES,
  guestSkillInventorySchema,
  type GuestSkillAttachment,
  type GuestSkillInventory,
  type GuestSkillSelection,
} from "@/schemas/channel-guest-skills";
import {
  guestSkillsPath,
  guestSkillsQueryKey,
  useGuestSkills,
} from "./guest-skills-query";

export function GuestSkillExposure({ botId, onStage }: {
  readonly botId: string;
  readonly onStage?: (selection: GuestSkillSelection) => void;
}) {
  const query = useGuestSkills(botId);
  const { t } = useUiTranslation();
  if (query.isPending) {
    return <p className="type-secondary text-muted-foreground">{t("Loading attached skills…")}</p>;
  }
  if (query.isError) {
    return (
      <div className="space-y-2">
        <p role="alert" className="type-secondary text-destructive">
          {query.error.message}
        </p>
        <Button variant="outline" type="button" onClick={() => void query.refetch()}>
          {t("Reload attached skills")}
        </Button>
      </div>
    );
  }
  if (query.data.available.length === 0) {
    return <p className="type-secondary text-muted-foreground">{t("This agent has no enabled skills.")}</p>;
  }
  return (
    <SkillAudienceForm
      key={`${botId}:${query.data.agent_id ?? ""}:${query.data.revision}:${query.data.skills_revision}`}
      botId={botId}
      inventory={query.data}
      onStage={onStage}
    />
  );
}

function SkillAudienceForm({
  botId,
  inventory,
  onStage,
}: {
  readonly botId: string;
  readonly inventory: GuestSkillInventory;
  readonly onStage?: (selection: GuestSkillSelection) => void;
}) {
  const cache = useQueryClient();
  const [updates, setUpdates] = useState<Record<string, GuestSkillAttachment>>({});
  const { t } = useUiTranslation();
  const selection = (value: Record<string, GuestSkillAttachment>): GuestSkillSelection => ({
    ...(inventory.agent_id && { agent_id: inventory.agent_id }),
    expected_revision: inventory.revision,
    skills_revision: inventory.skills_revision,
    attachments: [
      ...inventory.attachments.filter((entry) => !value[entry.skill_id]),
      ...Object.values(value),
    ],
  });
  const save = useMutation({
    mutationFn: async () =>
      guestSkillInventorySchema.parse(await api.put(guestSkillsPath(botId), selection(updates))),
    onSuccess: (value) => {
      cache.setQueryData(guestSkillsQueryKey(botId), value);
    },
  });

  return (
    <section className="space-y-3">
      <h3 className="type-body-strong">{t("Skills available to guests")}</h3>
      <p className="type-secondary text-muted-foreground">
        {t("The bot may quote files it reads to the selected audience. Private files also require the publisher to authorize bot use in Ornn.")}
      </p>
      {inventory.available.map((pin) => {
        const attachment = updates[pin.skill_id] ??
          inventory.attachments.find((entry) => entry.skill_id === pin.skill_id) ?? {
            skill_id: pin.skill_id,
            audiences: [...GUEST_SKILL_AUDIENCES],
            all_requests_approved: false,
          };
        const setAttachment = (value: GuestSkillAttachment) => {
          const next = { ...updates, [pin.skill_id]: value };
          setUpdates(next);
          onStage?.(selection(next));
        };
        return (
          <fieldset key={pin.skill_id} className="space-y-2 rounded-md border p-3">
            <Label asChild className="px-1">
              <legend>{pin.name} {pin.version}</legend>
            </Label>
            <div className="flex flex-wrap gap-x-4 gap-y-2">
              {GUEST_SKILL_AUDIENCES.map((audience) => (
                <Label key={audience} className="flex items-center gap-2">
                  <Checkbox
                    checked={attachment.audiences.includes(audience)}
                    disabled={save.isPending}
                    onCheckedChange={(checked) =>
                      setAttachment({
                        ...attachment,
                        audiences: checked === true
                          ? [...attachment.audiences, audience]
                          : attachment.audiences.filter((entry) => entry !== audience),
                      })
                    }
                  />
                  {audience === "private" ? t("Private replies") : t("Shared replies")}
                </Label>
              ))}
            </div>
            {(inventory.standing_approval_skill_ids?.includes(pin.skill_id) ?? true) && (
              <Label className="flex items-center gap-2">
                <Checkbox
                  checked={attachment.all_requests_approved}
                  disabled={save.isPending}
                  onCheckedChange={(checked) =>
                    setAttachment({ ...attachment, all_requests_approved: checked === true })
                  }
                />
                {t("Approve reads of this exact version and its pinned dependencies under All Requests")}
              </Label>
            )}
          </fieldset>
        );
      })}
      {save.isError && (
        <p role="alert" className="type-secondary text-destructive">{save.error.message}</p>
      )}
      {!onStage && <Button
        type="button"
        variant="outline"
        disabled={save.isPending || Object.keys(updates).length === 0}
        onClick={() => save.mutate()}
      >
        {save.isPending ? t("Saving…") : t("Save skill access")}
      </Button>}
    </section>
  );
}
