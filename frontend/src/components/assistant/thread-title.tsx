import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { useState } from "react";
import { Check, PencilLine, X } from "lucide-react";
import { useAppForm } from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

/** Title metadata stays editable while the conversation runs. */
export function ThreadTitle({
  title,
  onRename,
  heading = false,
}: {
  readonly title: string;
  readonly onRename: (title: string) => Promise<void>;
  /** Shown as the page's heading rather than a muted label. */
  readonly heading?: boolean;
}) {
  useUiTranslation();
  const [editing, setEditing] = useState(false);
  const [error, setError] = useState<string>();
  const form = useAppForm({ defaultValues: { title } });
  if (!editing)
    return (
      <button
        type="button"
        aria-label={translateUi("Rename chat")}
        title={title}
        className={cn(
          "group flex min-w-0 max-w-full items-center gap-2 text-left",
          heading
            ? "type-card-title text-foreground"
            : "text-14 text-muted-foreground hover:text-foreground",
        )}
        onClick={() => {
          form.reset({ title });
          setError(undefined);
          setEditing(true);
        }}
      >
        <span className="truncate">{title}</span>
        <PencilLine
          aria-hidden
          className={cn(
            "h-3 w-3 shrink-0 opacity-50 group-hover:opacity-100",
            heading && "text-text-tertiary",
          )}
        />
      </button>
    );
  return (
    <form
      aria-label={translateUi("Rename chat title")}
      className={heading ? "min-w-0 w-full max-w-2xl" : "min-w-0 max-w-sm"}
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.preventDefault();
          setEditing(false);
        }
      }}
      onSubmit={form.handleSubmit(async (values) => {
        setError(undefined);
        try {
          await onRename(values.title.trim());
          setEditing(false);
        } catch {
          setError("Could not rename. Try again.");
        }
      })}
    >
      <div className="flex items-center gap-1">
        <Input
          autoFocus
          aria-label={translateUi("Chat title")}
          className={heading ? "h-8 min-w-0 flex-1 type-body " : undefined}
          maxLength={200}
          {...form.register("title")}
        />
        <Button
          type="submit"
          size="icon"
          variant="ghost"
          aria-label={translateUi("Save title")}
          disabled={
            !form.formState.isDirty ||
            !form.watch("title").trim() ||
            form.formState.isSubmitting
          }
        >
          <Check />
        </Button>
        <Button
          type="button"
          size="icon"
          variant="ghost"
          aria-label={translateUi("Cancel rename")}
          onClick={() => setEditing(false)}
        >
          <X />
        </Button>
      </div>
      {error && (
        <p role="alert" className="text-13 text-destructive">
          {translateUi(error)}
        </p>
      )}
    </form>
  );
}
