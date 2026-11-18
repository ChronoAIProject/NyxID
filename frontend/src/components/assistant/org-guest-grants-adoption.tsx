import { useState } from "react";
import { z } from "zod";
import { Button } from "@/components/ui/button";
import { GroupedList, GroupedRow } from "@/components/ui/grouped-list";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog";
import { useOrgGuestAdoption } from "@/hooks/use-org-specialist-channel";
import { orgGuestAdoptionSchema } from "@/schemas/org-specialist-channel";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";

type Review = z.infer<typeof orgGuestAdoptionSchema>;
export function OrgGuestGrantsAdoption({
  agentId,
  disabled,
}: {
  readonly agentId: string;
  readonly disabled: boolean;
}) {
  useUiTranslation();
  const { preview, adopt } = useOrgGuestAdoption(agentId, true);
  const [review, setReview] = useState<Review>();
  if (!preview.data?.pending && !preview.isError) return null;
  return (
    <GroupedList
      footer={
        <>
          <p>
            {translateUi(
              "Existing guest limits are kept. Review them in Grants.",
            )}
          </p>
          <p>
            {translateUi("Save any Grants edits before reviewing the handoff.")}
          </p>
          {preview.isError || adopt.error ? (
            <p role="alert">
              {translateUi(
                "Could not apply guest settings. Reload and review them again.",
              )}
            </p>
          ) : null}
        </>
      }
    >
      <GroupedRow>
        <Button
          type="button"
          variant="outline"
          disabled={
            disabled ||
            !preview.data?.ready ||
            preview.isFetching ||
            adopt.isPending
          }
          onClick={async () => {
            const fresh = await preview.refetch();
            if (fresh.data && !fresh.isError) {
              adopt.reset();
              setReview(fresh.data);
            }
          }}
        >
          {translateUi(
            "Use these guest settings for all linked organization bots",
          )}
        </Button>
      </GroupedRow>
      <Dialog
        open={Boolean(review)}
        onOpenChange={(open) => {
          if (!open) setReview(undefined);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {translateUi(
                "Use these guest settings for all linked organization bots",
              )}
            </DialogTitle>
            <DialogDescription>
              {translateUi(
                "This replaces each bot's existing service limits with the saved Grants below. Disabled bots will use these settings if enabled later. The organization pays.",
              )}
            </DialogDescription>
          </DialogHeader>
          <div className="max-h-80 space-y-3 overflow-y-auto">
            <p>
              {translateUi("Affected bots")}:{" "}
              {review?.bots.map((b) => b.name).join(", ")}
            </p>
            <ul className="space-y-1">
              {review?.services.map((service) => (
                <li key={service.id}>
                  {service.label === "Unavailable service"
                    ? translateUi("Unavailable service")
                    : service.label}
                  :{" "}
                  {translateUi(
                    {
                      none: "No guest access",
                      read: "Look things up only",
                      use: "Use, but not change or delete",
                      all: "Everything this agent can do",
                    }[service.access],
                  )}
                  {service.limited_operations
                    ? ` · ${translateUi("Selected operations only")}`
                    : ""}
                </li>
              ))}
            </ul>
            {!review?.services.length ? (
              <p>{translateUi("Model-only chat")}</p>
            ) : null}
          </div>
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => setReview(undefined)}
            >
              {translateUi("Cancel")}
            </Button>
            <Button
              type="button"
              disabled={disabled || adopt.isPending || !review}
              onClick={async () => {
                if (!review) return;
                try {
                  await adopt.mutateAsync(review.review);
                  setReview(undefined);
                } catch {
                  setReview(undefined);
                }
              }}
            >
              {translateUi("Apply reviewed guest settings")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </GroupedList>
  );
}
