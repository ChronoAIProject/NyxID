import { useRef } from "react";
import { useUiTranslation } from "@/i18n/use-ui-translation";
import { Bell } from "lucide-react";
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
              "relative mr-0.5 flex shrink-0 items-center justify-center rounded-lg text-text-tertiary transition-colors hover:bg-overlay hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring",
              active && "bg-overlay-strong text-foreground",
            )}
          >
            <span aria-hidden="true" className="relative flex h-4 w-4 items-center justify-center">
              <Bell className="h-full w-full" style={{ width: "100%", height: "100%" }} />
              {count > 0 ? (
                <span
                  aria-hidden="true"
                  data-notification-count=""
                  className={cn(
                    "absolute left-2 -top-1.5 flex h-3.5 min-w-3.5 items-center justify-center rounded-full px-[3px] type-count-badge tabular-nums ring-2 ring-sidebar",
                    needsDecision > 0
                      ? "bg-warning text-neutral-950 light:text-white"
                      : "bg-primary text-primary-foreground",
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
