import { useEffect, useState } from "react";
import { Check, Loader2, MessageSquareText, Send } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import type { InputAnswer } from "@/schemas/assistant-input";
import type { InputCardContentBlock } from "@/types/assistant";

export function InputCard({
  block,
  disabled = false,
  onResolve,
}: {
  readonly block: InputCardContentBlock;
  readonly disabled?: boolean;
  readonly onResolve: (answer: InputAnswer) => Promise<void> | void;
}) {
  const [selectedOptionIds, setSelectedOptionIds] = useState<string[]>([]);
  const [freeText, setFreeText] = useState("");
  const [pending, setPending] = useState<"selection" | "text" | null>(null);

  useEffect(() => {
    setSelectedOptionIds([]);
    setFreeText("");
    setPending(null);
  }, [block.request_id]);

  async function submit(answer: InputAnswer, mode: "selection" | "text") {
    if (disabled || block.status !== "pending" || pending !== null) return;
    setPending(mode);
    try {
      await onResolve(answer);
    } catch {
      // The mutation owns delivery errors and re-enables this card on failure.
    } finally {
      setPending(null);
    }
  }

  if (block.status !== "pending") {
    const resolved = block.status === "resolved";
    const submitted = block.status === "submitted";
    return (
      <section
        className={`rounded-lg border p-4 ${
          resolved
            ? "border-success/30 bg-success/[0.06]"
            : submitted
              ? "border-warning/30 bg-warning/[0.06]"
              : "border-border bg-overlay"
        }`}
      >
        <div className="flex items-center gap-2 text-[12px] font-semibold text-foreground">
          {submitted ? (
            <Loader2 className="h-4 w-4 animate-spin text-warning" />
          ) : (
            <Check
              className={`h-4 w-4 ${resolved ? "text-success" : "text-muted-foreground"}`}
            />
          )}
          {resolved
            ? "Answer recorded"
            : submitted
              ? "Answer sent"
              : "Input cancelled"}
        </div>
        <p className="mt-2 text-[12px] leading-relaxed text-muted-foreground">
          {block.prompt}
        </p>
        {submitted ? (
          <p className="mt-1.5 text-[11px] text-text-tertiary">
            Waiting for committed confirmation.
          </p>
        ) : null}
      </section>
    );
  }

  const busy = pending !== null;
  const normalizedText = freeText.trim();
  return (
    <section className="overflow-hidden rounded-lg border border-border bg-card shadow-sm">
      <div className="flex items-start gap-3 border-b border-border px-4 py-3.5">
        <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-nyx-secondary-400/30 bg-nyx-secondary-400/10">
          <MessageSquareText className="h-4 w-4 text-nyx-secondary-400" />
        </div>
        <div className="min-w-0 flex-1">
          <h3 className="text-[13px] font-semibold text-foreground">
            Input required
          </h3>
          <p className="mt-1 text-[12px] leading-relaxed text-muted-foreground">
            {block.prompt}
          </p>
        </div>
      </div>

      {block.options.length > 0 ? (
        <div className="grid gap-2 px-4 py-3.5">
          {block.options.map((option) => {
            const checked = selectedOptionIds.includes(option.option_id);
            return (
              <label
                key={option.option_id}
                className="flex cursor-pointer items-start gap-2.5 rounded-lg border border-border px-3 py-2.5 text-[12px] hover:bg-muted"
              >
                <input
                  checked={checked}
                  disabled={busy || disabled}
                  name={`assistant-input-${block.request_id}`}
                  onChange={(event) => {
                    setSelectedOptionIds((current) =>
                      block.multi_select
                        ? event.target.checked
                          ? [...new Set([...current, option.option_id])]
                          : current.filter((id) => id !== option.option_id)
                        : event.target.checked
                          ? [option.option_id]
                          : [],
                    );
                  }}
                  type={block.multi_select ? "checkbox" : "radio"}
                  value={option.option_id}
                />
                <span className="min-w-0">
                  <span className="block font-medium text-foreground">
                    {option.label}
                  </span>
                  {option.description ? (
                    <span className="mt-0.5 block text-[11px] text-muted-foreground">
                      {option.description}
                    </span>
                  ) : null}
                </span>
              </label>
            );
          })}
          <Button
            type="button"
            size="sm"
            className="mt-1 w-fit"
            disabled={busy || disabled || selectedOptionIds.length === 0}
            onClick={() => void submit({ selectedOptionIds }, "selection")}
          >
            {pending === "selection" ? (
              <Loader2 className="animate-spin" />
            ) : (
              <Send />
            )}
            Submit
          </Button>
        </div>
      ) : null}

      {block.allow_free_text ? (
        <form
          className="flex gap-2 border-t border-border px-4 py-3"
          onSubmit={(event) => {
            event.preventDefault();
            if (normalizedText)
              void submit({ freeText: normalizedText }, "text");
          }}
        >
          <Input
            aria-label="Answer"
            disabled={busy || disabled}
            maxLength={32_768}
            onChange={(event) => setFreeText(event.target.value)}
            value={freeText}
          />
          <Button
            type="submit"
            size="icon"
            disabled={busy || disabled || !normalizedText}
            title="Submit answer"
          >
            {pending === "text" ? (
              <Loader2 className="animate-spin" />
            ) : (
              <Send />
            )}
            <span className="sr-only">Submit answer</span>
          </Button>
        </form>
      ) : null}
    </section>
  );
}
