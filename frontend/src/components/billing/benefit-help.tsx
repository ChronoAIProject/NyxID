import { getFormattingLocale } from "@/lib/formatting-locale";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";
import { useState, type ReactNode } from "react";
import { Info } from "lucide-react";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import type { UserAllowanceBalance } from "@/schemas/billing-credits";

export function BenefitHelp({
  label,
  children,
}: {
  label: string;
  children: ReactNode;
}) {
  const { t } = useUiTranslation();
  const [open, setOpen] = useState(false);
  return (
    <Tooltip open={open} onOpenChange={setOpen} delayDuration={150}>
      <TooltipTrigger asChild>
        <button
          type="button"
          className="benefit-help-trigger"
          aria-label={t("About {{value}}", { value: label.toLowerCase() })}
          onClick={(event) => {
            event.preventDefault();
            event.stopPropagation();
            setOpen(true);
          }}
          onKeyDown={(event) => {
            if (event.key === "Enter" || event.key === " ")
              event.stopPropagation();
          }}
        >
          <Info size={12} />
        </button>
      </TooltipTrigger>
      <TooltipContent
        className="billing-benefit-help-content"
        side="top"
        collisionPadding={16}
      >
        <strong>{label}</strong>
        {children}
      </TooltipContent>
    </Tooltip>
  );
}

function allowanceCadence(balances: readonly UserAllowanceBalance[]) {
  const cadences = [
    ...new Set(balances.map((row) => row.allowance.recurrence)),
  ];
  if (cadences.length === 1 && cadences[0] === "daily") {
    const resets = [...new Set(balances.map((row) => row.period_end))];
    if (resets.length === 1 && resets[0]) {
      const time = new Intl.DateTimeFormat(getFormattingLocale(), {
        timeZoneName: "short",
        hour: "2-digit",
        minute: "2-digit",
        hour12: false,
      }).format(new Date(resets[0]));
      return translateUi("Your allowances reset daily at {{time}}. Unused units do not roll over.", { time });
    }
    return translateUi("Your allowances reset daily. Open Details for each reset time. Unused units do not roll over.");
  }
  if (cadences.length === 1 && cadences[0] === "one_time")
    return translateUi("Your allowances are one-time allocations and do not reset.");
  if (cadences.length === 1)
    return translateUi("Your allowances reset {{recurrence}}. Unused units do not roll over. Open Details for each reset time.", { recurrence: translateUi(cadences[0]!) });
  return translateUi("These allowances have different reset schedules. Open Details for each allowance's recurrence and expiry.");
}

export function FreeUsageHelp({
  balances,
}: {
  balances: readonly UserAllowanceBalance[];
}) {
  const { t } = useUiTranslation();
  return (
    <BenefitHelp label={t("Free usage")}>
      <p>{t("Included units for this service's platform usage. Each token class, image, or request allowance has its own limit and cannot cover another unit. Matching free usage is applied before credit grants and your wallet.")}</p>
      <p>
        {allowanceCadence(balances)}{t(" After an allowance is exhausted, eligible grants or wallet credits cover further charges.")}</p>
    </BenefitHelp>
  );
}
