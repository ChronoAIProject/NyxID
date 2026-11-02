import type { ComponentProps } from "react";
import { cn } from "@/lib/utils";

export function UserMessageBubble({ className, ...props }: ComponentProps<"div">) {
  return (
    <div
      className={cn(
        "rounded-[18px] rounded-br-[4px] bg-chat-bubble px-3 py-2 text-12 leading-relaxed text-chat-bubble-foreground whitespace-pre-wrap break-words",
        className,
      )}
      {...props}
    />
  );
}
