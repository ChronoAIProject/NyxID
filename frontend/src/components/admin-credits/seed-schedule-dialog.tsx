import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";
import { useRef } from "react";
import { toast } from "sonner";
import { useSeedCreditSchedule } from "@/hooks/use-billing-credits";
import { exactCredits, formatExactCredits } from "@/lib/credits";
import { ApiError } from "@/lib/api-client";
import type { CreditSchedule } from "@/schemas/billing-credits";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

export function SeedScheduleDialog({
  schedule,
  onClose,
}: {
  readonly schedule: CreditSchedule;
  readonly onClose: () => void;
}) {
  useUiTranslation();
  const mutation = useSeedCreditSchedule();
  const inFlight = useRef(false);
  const credits =
    (schedule.current_period
      ? exactCredits(
          schedule.current_period.amount,
          schedule.current_period.amount_micros,
          "micros",
        )
      : exactCredits(schedule.amount, schedule.amount_credits, "whole")) ?? "0";

  async function seed() {
    if (inFlight.current) return;
    inFlight.current = true;
    try {
      const result = await mutation.mutateAsync({
        id: schedule.id,
        updatedAt: schedule.updated_at,
        periodStart: schedule.current_period?.start,
      });
      toast.success(
        result.pending > 0
          ? translateUi(
              "Granted credits to {{created}} users; {{existing}} already received this period's grant. {{pending}} awaiting activation.",
              {
                created: result.created,
                existing: result.existing,
                pending: result.pending,
              },
            )
          : translateUi(
              "Granted credits to {{created}} users; {{existing}} already received this period's grant.",
              { created: result.created, existing: result.existing },
            ),
      );
      onClose();
    } catch (error) {
      toast.error(
        translateUi(
          error instanceof ApiError && error.status === 409
            ? "The schedule or period changed. Close this dialog and review the refreshed schedule before trying again."
            : "Granting missing credits did not finish. Retry safely; users already granted are skipped.",
        ),
      );
    } finally {
      inFlight.current = false;
    }
  }

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !inFlight.current) onClose();
      }}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{translateUi("Grant missing credits")}</DialogTitle>
          <DialogDescription>
            {translateUi(
              "Give {{credits}} credits to each currently eligible user who missed this schedule's current-period grant. Existing users are included.",
              { credits: formatExactCredits(credits, 12) },
            )}
          </DialogDescription>
        </DialogHeader>
        <p className="text-14 text-muted-foreground">
          {translateUi(
            "The current period's recipient rules, service limits and expiry apply. Users who already received its grant are skipped, including grants that have been spent, expired or reclaimed. Earlier periods are not backfilled. Retrying will not grant credits twice.",
          )}
        </p>
        <p className="break-all text-13 text-muted-foreground">
          {translateUi("Schedule {{id}}", { id: schedule.id })}
        </p>
        <DialogFooter>
          <Button
            variant="outline"
            disabled={mutation.isPending}
            onClick={onClose}
          >
            {translateUi("Cancel")}
          </Button>
          <Button isLoading={mutation.isPending} onClick={() => void seed()}>
            {translateUi("Grant missing credits")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
