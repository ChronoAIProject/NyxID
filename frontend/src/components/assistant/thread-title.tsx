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
  const [editing, setEditing] = useState(false);
  const [error, setError] = useState<string>();
  const form = useAppForm({ defaultValues: { title } });
  if (!editing)
    return (
      <button
        type="button"
        aria-label="Rename chat"
        title={title}
        className={cn(
          "group flex min-w-0 max-w-full items-center gap-2 text-left",
          heading
            ? "text-15 font-semibold text-foreground"
            : "text-12 text-muted-foreground hover:text-foreground",
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
      aria-label="Rename chat title"
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
          aria-label="Chat title"
          className={heading ? "h-8 min-w-0 flex-1 text-15 font-semibold" : undefined}
          maxLength={200}
          {...form.register("title")}
        />
        <Button
          type="submit"
          size="icon"
          variant="ghost"
          aria-label="Save title"
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
          aria-label="Cancel rename"
          onClick={() => setEditing(false)}
        >
          <X />
        </Button>
      </div>
      {error && (
        <p role="alert" className="text-11 text-destructive">
          {error}
        </p>
      )}
    </form>
  );
}
