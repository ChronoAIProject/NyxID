import { useRef } from "react";
import { useUiTranslation } from "@/i18n/use-ui-translation";
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
 * the Notifications feature; waiting decisions tint the single count pill amber.
 */
export function NotificationsBell() {
  useUiTranslation();
  const enabled = useNotificationsEnabled();
  const summary = useNotificationSummary(enabled);
  const { open, current } = useAccountPanel();
  const trigger = useRef<HTMLButtonElement>(null);
  if (!enabled) return null;
  const unread = summary.data?.unread_count ?? 0;
  const needsDecision = summary.data?.needs_decision_count ?? 0;
  const count = unread > 0 ? unread : needsDecision;
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
            <span aria-hidden="true" className="relative flex h-4 w-4 items-center justify-center">
              <Bell className="h-4 w-4" />
              {count > 0 ? (
                <span
                  aria-hidden="true"
                  data-notification-count=""
                  className={cn(
                    badgeVariants({ variant: needsDecision > 0 ? "warning" : "secondary" }),
                    "absolute right-0 top-0 min-w-4 -translate-y-1/2 translate-x-1/2 justify-center rounded-full border-hairline px-1 py-0 tabular-nums",
                  )}
                >
                  {notificationBadgeText(count)}
                </span>
              ) : null}
            </span>
          </button>
        </TooltipTrigger>
        <TooltipContent side="bottom" sideOffset={6}>
          {label}
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}
