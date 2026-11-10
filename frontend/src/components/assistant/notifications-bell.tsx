import { useRef } from "react";
import { Bell } from "lucide-react";
import { badgeVariants } from "@/components/ui/badge";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { useAccountPanel } from "@/hooks/use-account-panel";
import {
  useNotificationSummary,
  useNotificationsEnabled,
} from "@/hooks/use-assistant-notifications";
import {
  notificationBadgeText,
  notificationBellLabel,
} from "@/lib/assistant/notification-presentation";
import { cn } from "@/lib/utils";

/**
 * The bell beside Search in the assistant header. Hidden entirely without
 * the Notifications feature; the badge turns amber while a decision waits.
 */
export function NotificationsBell() {
  const enabled = useNotificationsEnabled();
  const summary = useNotificationSummary(enabled);
  const { open, current } = useAccountPanel();
  const trigger = useRef<HTMLButtonElement>(null);
  if (!enabled) return null;
  const unread = summary.data?.unread_count ?? 0;
  const needsDecision = summary.data?.needs_decision_count ?? 0;
  const label = notificationBellLabel(unread, needsDecision);
  const active = current.panel === "notifications";
  return (
    <TooltipProvider delayDuration={200}>
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            ref={trigger}
            data-icon-button=""
            data-tour="notifications"
            type="button"
            aria-label={label}
            aria-haspopup="dialog"
            aria-expanded={active}
            onClick={() => void open("notifications", {}, trigger.current)}
            className={cn(
              "relative flex shrink-0 items-center justify-center rounded-lg text-text-tertiary transition-colors hover:bg-overlay hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring",
              active && "bg-overlay-strong text-foreground",
            )}
          >
            <Bell aria-hidden="true" className="h-4 w-4" />
            {unread > 0 ? (
              <span
                aria-hidden="true"
                data-needs-decision={needsDecision > 0 ? "" : undefined}
                className={cn(
                  badgeVariants({ variant: needsDecision > 0 ? "warning" : "default" }),
                  "absolute -right-1 -top-1 min-w-4 justify-center rounded-full px-1 py-0 tabular-nums",
                )}
              >
                {notificationBadgeText(unread)}
              </span>
            ) : null}
          </button>
        </TooltipTrigger>
        <TooltipContent side="bottom" sideOffset={6}>
          {label}
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}
