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
        `Granted credits to ${String(result.created)} users; ${String(result.existing)} already received this period's grant.${result.pending > 0 ? ` ${String(result.pending)} awaiting activation.` : ""}`,
      );
      onClose();
    } catch (error) {
      toast.error(
        error instanceof ApiError && error.status === 409
          ? "The schedule or period changed. Close this dialog and review the refreshed schedule before trying again."
          : "Granting missing credits did not finish. Retry safely; users already granted are skipped.",
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
          <DialogTitle>Grant missing credits</DialogTitle>
          <DialogDescription>
            Give {formatExactCredits(credits, 12)} credits to each currently
            eligible user who missed this schedule&apos;s current-period grant.
            Existing users are included.
          </DialogDescription>
        </DialogHeader>
        <p className="text-14 text-muted-foreground">
          The current period&apos;s recipient rules, service limits and expiry
          apply. Users who already received its grant are skipped, including
          grants that have been spent, expired or reclaimed. Earlier periods are
          not backfilled. Retrying will not grant credits twice.
        </p>
        <p className="break-all text-13 text-muted-foreground">
          Schedule {schedule.id}
        </p>
        <DialogFooter>
          <Button
            variant="outline"
            disabled={mutation.isPending}
            onClick={onClose}
          >
            Cancel
          </Button>
          <Button isLoading={mutation.isPending} onClick={() => void seed()}>
            Grant missing credits
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
