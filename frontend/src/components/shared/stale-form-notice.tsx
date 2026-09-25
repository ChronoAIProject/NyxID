import { Button } from "@/components/ui/button";

export function StaleFormNotice({
  onReload,
}: {
  readonly onReload: () => void;
}) {
  return (
    <div
      role="alert"
      className="space-y-2 rounded-md border border-border p-3 text-sm"
    >
      <p>
        Saved values changed while you were editing. Load the latest values
        before saving.
      </p>
      <Button type="button" variant="outline" onClick={onReload}>
        Load latest values (discard edits)
      </Button>
    </div>
  );
}
