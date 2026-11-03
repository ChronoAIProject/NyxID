import { messageTime } from "@/lib/assistant/message-time";
import { cn } from "@/lib/utils";

/**
 * When a message was sent: its local day and month (and year, outside the
 * current one), with the full date, time and zone on hover. `reveal` keeps it
 * hidden until its `group/message` ancestor is hovered or holds focus;
 * devices without hover always show it.
 */
export function MessageTime({
  value,
  reveal = false,
  className,
}: {
  readonly value: number | string | null | undefined;
  readonly reveal?: boolean;
  readonly className?: string;
}) {
  const time = messageTime(value);
  if (!time) return null;
  return (
    <time
      dateTime={time.iso}
      title={time.label}
      className={cn(
        "max-w-full whitespace-normal text-11 text-text-tertiary [overflow-wrap:anywhere]",
        reveal &&
          "opacity-0 transition-opacity group-hover/message:opacity-100 group-focus-within/message:opacity-100 [@media(hover:none)]:opacity-100",
        className,
      )}
    >
      {time.short}
    </time>
  );
}
