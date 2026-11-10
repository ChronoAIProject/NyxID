import { useAppTranslation } from "@/i18n/use-app-translation";
import { cn } from "@/lib/utils";

export function DecisionContinuation({
  className,
}: {
  readonly className?: string;
}) {
  const { t } = useAppTranslation();
  return (
    <p
      role="status"
      className={cn("px-[7px] text-11 text-muted-foreground", className)}
    >
      {t("Continuing after your decision")}
    </p>
  );
}
