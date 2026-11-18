import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { ACCOUNT_DELETION_DISCLOSURE } from "@/lib/account-deletion-policy";

/** Keep both owner account-deletion confirmations on the same disclosed policy. */
export function AccountDeletionDisclosure() {
  useUiTranslation();
  return (
    <div
      className="space-y-3 text-14"
      role="note"
      aria-label={translateUi("Machine data deletion and retained records")}
    >
      {ACCOUNT_DELETION_DISCLOSURE.split("\n\n").map((paragraph, index) => (
        <p
          key={paragraph}
          className={
            index === 0
              ? "rounded-lg border border-destructive/40 bg-destructive/5 p-3"
              : "text-muted-foreground"
          }
        >
          {paragraph}
        </p>
      ))}
    </div>
  );
}
