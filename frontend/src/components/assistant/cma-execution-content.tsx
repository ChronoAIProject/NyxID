import { useId, useState, type ReactNode } from "react";
import { Bot, ChevronDown, Copy, Link2, MessageSquare } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { UserMessageBubble } from "@/components/ui/user-message-bubble";
import { TextBlock } from "@/components/assistant/blocks/text-block";
import { cn, copyToClipboard } from "@/lib/utils";
import { cmaReadableLabel } from "@/lib/assistant/cma-display";
import { toast } from "sonner";

export function CmaStatus({ status }: { readonly status: string }) {
  const label =
    status === "incomplete"
      ? "Not completed"
      : status === "waiting_approval"
        ? "Waiting for confirmation"
        : status === "unknown"
          ? "Status unavailable"
          : cmaReadableLabel(status);
  const variant = ["completed", "succeeded", "ready"].includes(status)
    ? "success"
    : ["failed", "error"].includes(status)
      ? "destructive"
      : [
            "incomplete",
            "interrupted",
            "waiting_approval",
            "awaiting_approval",
            "blocked",
          ].includes(status)
        ? "warning"
        : ["in_progress", "working", "queued", "provisioning"].includes(status)
          ? "info"
          : "secondary";
  return <Badge variant={variant}>{label}</Badge>;
}

export function CmaTime({
  value,
  timeOnly = false,
}: {
  readonly value?: string | null;
  readonly timeOnly?: boolean;
}) {
  if (!value || Number.isNaN(Date.parse(value))) return null;
  return (
    <time
      dateTime={value}
      title={new Date(value).toLocaleString()}
      className="shrink-0 type-meta text-text-tertiary"
    >
      {new Date(value).toLocaleString(undefined, {
        ...(!timeOnly && ({ month: "short", day: "numeric" } as const)),
        hour: "2-digit",
        minute: "2-digit",
      })}
    </time>
  );
}

export function CmaDisclosure({
  title,
  count,
  children,
}: {
  readonly title: string;
  readonly count?: number;
  readonly children: ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const id = useId();
  return (
    <section className="rounded-xl border border-hairline bg-card">
      <button
        type="button"
        aria-expanded={open}
        aria-controls={id}
        onClick={() => setOpen(!open)}
        className="flex w-full items-center gap-2 rounded-xl p-4 text-left type-button hover:bg-overlay focus-visible:outline focus-visible:outline-2 focus-visible:outline-ring"
      >
        <ChevronDown
          aria-hidden
          className={cn(
            "size-3.5 text-text-tertiary transition-transform",
            !open && "-rotate-90",
          )}
        />
        {title}
        {count !== undefined && (
          <span className="ml-auto type-secondary text-text-tertiary">
            {count} records
          </span>
        )}
      </button>
      {open && (
        <div
          id={id}
          role="region"
          aria-label={title}
          className="border-t border-hairline p-4"
        >
          {children}
        </div>
      )}
    </section>
  );
}

export function CmaMessage({
  role,
  text,
  time,
  assistantName = "Cloud assistant",
  linked = false,
}: {
  readonly role: string;
  readonly text: string;
  readonly time?: string | null;
  readonly assistantName?: string;
  readonly linked?: boolean;
}) {
  const request = role === "user";
  const Icon = request ? MessageSquare : Bot;
  const id = useId();
  const [expanded, setExpanded] = useState(false);
  const long = text.length > 360 || text.split("\n").length > 6;
  return (
    <article
      className={cn(
        "flex min-w-0 items-start gap-2.5 sm:gap-3",
        request && "flex-row-reverse",
      )}
    >
      <span
        className={cn(
          "flex size-7 shrink-0 items-center justify-center rounded-full border border-hairline bg-card text-text-tertiary",
          request ? "mt-7" : "text-nyx-secondary-400",
        )}
      >
        <Icon aria-hidden className="size-3.5" />
      </span>
      <div
        className={cn(
          "min-w-0",
          request ? "max-w-[87%] sm:max-w-[82%]" : "flex-1",
        )}
      >
        <div
          className={cn(
            "mb-2 flex flex-wrap items-center gap-2 type-meta text-text-tertiary",
            request && "justify-end",
          )}
        >
          {linked && (
            <span className="mr-auto inline-flex items-center gap-1 text-nyx-secondary-400">
              <Link2 aria-hidden className="size-3" />
              From your chat
            </span>
          )}
          <h3 className="type-group-label text-muted-foreground">
            {request ? "Work request" : assistantName}
          </h3>
          <CmaTime value={time} timeOnly />
        </div>
        {request ? (
          <UserMessageBubble
            className={cn(
              "rounded-2xl rounded-tr-sm rounded-br-2xl px-4 py-3 type-chat-message",
              linked && "ring-1 ring-ring",
            )}
          >
            <p
              id={id}
              className={cn(
                "[overflow-wrap:anywhere]",
                long && !expanded && "line-clamp-4",
              )}
            >
              {text}
            </p>
            {long && (
              <button
                type="button"
                aria-controls={id}
                aria-expanded={expanded}
                onClick={() => setExpanded(!expanded)}
                className="mt-2 inline-flex items-center gap-1 rounded type-button text-text-tertiary hover:text-foreground focus-visible:outline focus-visible:outline-2 focus-visible:outline-ring"
              >
                {expanded ? "Show less" : "Show full message"}
                <ChevronDown
                  aria-hidden
                  className={cn("size-3", expanded && "rotate-180")}
                />
              </button>
            )}
          </UserMessageBubble>
        ) : (
          <div className="space-y-2">
            <TextBlock text={text} />
            {text && (
              <Button
                variant="ghost"
                size="icon"
                className="size-7 text-text-tertiary"
                aria-label="Copy reply"
                onClick={() => {
                  void copyToClipboard(text).then(
                    () => toast.success("Reply copied"),
                    () => toast.error("Failed to copy reply"),
                  );
                }}
              >
                <Copy aria-hidden />
              </Button>
            )}
          </div>
        )}
      </div>
    </article>
  );
}

export function CmaEmptyState({
  title,
  text,
}: {
  readonly title: string;
  readonly text: string;
}) {
  return (
    <div
      role="status"
      className="rounded-xl border border-dashed border-border px-5 py-10 text-center"
    >
      <MessageSquare
        aria-hidden
        className="mx-auto mb-3 size-6 text-text-tertiary"
      />
      <p className="type-secondary-emphasis">{title}</p>
      <p className="mx-auto mt-2 max-w-md type-secondary text-text-tertiary">
        {text}
      </p>
    </div>
  );
}
