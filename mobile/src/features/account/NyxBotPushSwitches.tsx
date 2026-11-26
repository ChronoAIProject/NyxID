import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp } from "../../i18n/appTranslation";
import { Switch } from "react-native";
import {
  AtSign,
  Bot,
  CalendarClock,
  CircleAlert,
  CircleHelp,
  MessageSquare,
  type LucideIcon,
} from "lucide-react-native";
import {
  SettingsGroup,
  SettingsRow,
} from "../../components/settings/SettingsList";
import { useTheme } from "../../theme/ThemeContext";
import {
  DEFAULT_NOTIFICATION_PUSH,
  type NotificationKind,
  type NotificationPushPreferences,
} from "../notifications/notificationContracts";

/** One switch per closed notification kind (SPEC D9); all default off. */
export const NYXBOT_PUSH_ROWS: {
  kind: NotificationKind;
  label: string;
  detail: string;
  Icon: LucideIcon;
}[] = [
  {
    kind: "automation_result",
    get label() {
      return translateApp("Automation results");
    },
    get detail() {
      return translateApp("A scheduled or webhook task finished");
    },
    Icon: CalendarClock,
  },
  {
    kind: "automation_problem",
    get label() {
      return translateApp("Automation problems");
    },
    get detail() {
      return translateApp("A task failed, was skipped or paused");
    },
    Icon: CircleAlert,
  },
  {
    kind: "needs_decision",
    get label() {
      return translateApp("Decisions needed");
    },
    get detail() {
      return translateApp("An action card is waiting for you");
    },
    Icon: CircleHelp,
  },
  {
    kind: "agent_update",
    get label() {
      return translateApp("Agent updates");
    },
    get detail() {
      return translateApp("Replies in threads you did not start here");
    },
    Icon: Bot,
  },
  {
    kind: "chat_update",
    get label() {
      return translateApp("Chat updates");
    },
    get detail() {
      return translateApp("Replies in your channel bot chats");
    },
    Icon: MessageSquare,
  },
  {
    kind: "group_mention",
    get label() {
      return translateApp("Group mentions");
    },
    get detail() {
      return translateApp("Someone mentioned you in a group");
    },
    Icon: AtSign,
  },
];

export const NYXBOT_PUSH_FOOTER_READY =
  "Pushed once when a new notification arrives; the feed in Notifications always keeps them.";
export const NYXBOT_PUSH_FOOTER_NO_DEVICE =
  "Turn on push notifications above to receive these on this device; the feed in Notifications always keeps them.";

/**
 * The six NyxBot push switches. Every change saves the complete object so one
 * toggle can never drop another switch's saved value (the API replaces the
 * slice). Switches stay disabled until the saved values are known and while
 * a save is in flight.
 */
export function NyxBotPushSwitches({
  preferences,
  saving,
  pushReady,
  onSave,
}: {
  /** Saved switches; undefined while they load. */
  preferences: NotificationPushPreferences | undefined;
  saving: boolean;
  /** This device is registered and push delivery is on. */
  pushReady: boolean;
  onSave: (next: NotificationPushPreferences) => void;
}) {
  useAppLanguage();
  const { colors } = useTheme();
  const switchTrack = { false: colors.borderSoft, true: colors.success };
  const disabled = !preferences || saving;
  return (
    <SettingsGroup
      title={translateApp("NyxBot notifications")}
      footer={pushReady ? translateApp(NYXBOT_PUSH_FOOTER_READY)
          : translateApp(NYXBOT_PUSH_FOOTER_NO_DEVICE)
      }
    >
      {NYXBOT_PUSH_ROWS.map((row) => (
        <SettingsRow
          key={row.kind}
          icon={row.Icon}
          label={row.label}
          detail={row.detail}
          accessory={
            <Switch
              accessibilityLabel={row.label}
              value={preferences?.[row.kind] ?? false}
              onValueChange={(value) => {
                // Never save from an unknown baseline or over a save in flight.
                if (disabled) return;
                onSave({
                  ...(preferences ?? DEFAULT_NOTIFICATION_PUSH),
                  [row.kind]: value,
                });
              }}
              disabled={disabled}
              trackColor={switchTrack}
            />
          }
        />
      ))}
    </SettingsGroup>
  );
}
