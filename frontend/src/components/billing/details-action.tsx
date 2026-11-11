import { useUiTranslation } from "@/i18n/use-ui-translation";
import { ChevronDown } from "lucide-react";

/** The "Details ▾" affordance pinned top-right of every benefit summary. */
export function DetailsAction() {
  const { t } = useUiTranslation();
  return (
    <span className="compact-benefit-action">{t("Details ")}<ChevronDown size={13} className="disclosure-arrow" />
    </span>
  );
}
