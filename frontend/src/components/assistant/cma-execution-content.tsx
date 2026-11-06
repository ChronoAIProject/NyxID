import { useId, useState, type ReactNode } from "react";
import { Bot, ChevronDown, MessageSquare } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { TextBlock } from "@/components/assistant/blocks/text-block";
import { cn } from "@/lib/utils";
import { cmaReadableLabel } from "@/lib/assistant/cma-display";

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

export function CmaTime({ value }: { readonly value?: string | null }) {
  if (!value || Number.isNaN(Date.parse(value))) return null;
  return (
    <time dateTime={value} className="shrink-0 text-11 text-text-tertiary">
      {new Date(value).toLocaleString(undefined, {
        month: "short",
        day: "numeric",
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
        className="flex w-full items-center gap-2 rounded-xl p-4 text-left text-12 font-medium hover:bg-overlay focus-visible:outline focus-visible:outline-2 focus-visible:outline-ring"
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
          <span className="ml-auto text-11 font-normal text-text-tertiary">
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
}: {
  readonly role: string;
  readonly text: string;
  readonly time?: string | null;
}) {
  const request = role === "user";
  const Icon = request ? MessageSquare : Bot;
  return (
    <article
      className={cn(
        "min-w-0 rounded-lg p-4",
        request ? "border border-hairline bg-overlay" : "bg-card",
      )}
    >
      <div className="mb-3 flex flex-wrap items-center gap-2">
        <Icon aria-hidden className="size-3.5 text-text-tertiary" />
        <h3 className="text-12 font-medium">
          {request ? "Work request" : "Assistant reply"}
        </h3>
        <span className="ml-auto">
          <CmaTime value={time} />
        </span>
      </div>
      {request ? (
        <p className="whitespace-pre-wrap break-words text-13 leading-relaxed [overflow-wrap:anywhere]">
          {text}
        </p>
      ) : (
        <div className="[&>div]:text-13">
          <TextBlock text={text} />
        </div>
      )}
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
      <p className="text-13 font-medium">{title}</p>
      <p className="mx-auto mt-2 max-w-md text-12 leading-relaxed text-text-tertiary">
        {text}
      </p>
    </div>
  );
}
