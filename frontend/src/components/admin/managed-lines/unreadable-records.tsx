import { AlertTriangle } from "lucide-react";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";

/** Stored rows the server skipped because they could not be read. */
export function UnreadableRecords({ ids }: { readonly ids: readonly string[] }) {
  useUiTranslation();
  if (ids.length === 0) return null;
  return (
    <div
      role="alert"
      className="space-y-2 rounded-lg border border-warning/40 bg-warning/10 px-3 py-2 text-14"
    >
      <p className="flex items-start gap-2 text-warning">
        <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
        {translateUi("These stored records could not be read and were skipped. Repair them so they appear here again:")}
      </p>
      <ul className="space-y-1 break-all font-mono text-muted-foreground">
        {ids.map((id) => (
          <li key={id}>{id}</li>
        ))}
      </ul>
    </div>
  );
}
