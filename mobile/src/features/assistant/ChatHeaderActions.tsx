import {
  Bell,
  Info,
  MoreHorizontal,
  Settings,
  SlidersHorizontal,
  SquarePen,
} from "lucide-react-native";
import { FloatingItem } from "../../components/FloatingControl";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { useTheme } from "../../theme/ThemeContext";
import {
  notificationBadge,
  notificationBellLabel,
} from "../notifications/notificationPresentation";
import { ChatHeaderMenu, type ChatMenuOption } from "./ChatHeaderMenu";

export function ChatHeaderActions({
  newChatDisabled,
  menuDisabled,
  menuScope,
  onNewChat,
  onDetails,
  onSettings,
  notifications,
  onVoicePreferences,
}: {
  newChatDisabled?: boolean;
  menuDisabled?: boolean;
  menuScope: string;
  onNewChat: () => void;
  onDetails: () => void;
  onSettings: () => void;
  notifications?: {
    unread: number;
    needsDecision: number;
    disabled?: boolean;
    onPress: () => void;
  };
  onVoicePreferences?: () => void;
}) {
  const { t } = useAppLanguage();
  const { colors } = useTheme();
  const notificationCount = notifications
    ? notificationBadge(notifications.unread || notifications.needsDecision)
    : undefined;
  const options: ChatMenuOption[] = [
    { id: "details", label: t("Chat details"), Icon: Info, onPress: onDetails },
    ...(notifications
      ? [
          {
            id: "notifications",
            label: t("Notifications"),
            accessibilityLabel: notificationBellLabel(
              notifications.unread,
              notifications.needsDecision,
            ),
            badge: notificationCount,
            badgeColor:
              notifications.needsDecision > 0
                ? colors.warningTone.text
                : colors.textSecondary,
            Icon: Bell,
            disabled: notifications.disabled,
            onPress: notifications.onPress,
          },
        ]
      : []),
    {
      id: "settings",
      label: t("Settings"),
      Icon: Settings,
      onPress: onSettings,
    },
    ...(onVoicePreferences
      ? [
          {
            id: "voice-preferences",
            label: t("Voice preferences"),
            Icon: SlidersHorizontal,
            onPress: onVoicePreferences,
          },
        ]
      : []),
  ];
  return (
    <ChatHeaderMenu
      scope={menuScope}
      disabled={menuDisabled}
      options={options}
      controls={(openMenu, trigger) => (
        <>
          <FloatingItem
            accessibilityLabel={t("New chat")}
            onPress={onNewChat}
            disabled={newChatDisabled}
          >
            <SquarePen size={21} color={colors.textPrimary} />
          </FloatingItem>
          <FloatingItem
            ref={trigger}
            accessibilityLabel={t("Chat options")}
            accessibilityState={{ expanded: false }}
            onPress={openMenu}
            disabled={menuDisabled}
          >
            <MoreHorizontal size={23} color={colors.textPrimary} />
          </FloatingItem>
        </>
      )}
    />
  );
}

export function CrewHeaderActions({
  scope,
  disabled,
  onSettings,
}: {
  scope: string;
  disabled?: boolean;
  onSettings: () => void;
}) {
  const { t } = useAppLanguage();
  const { colors } = useTheme();
  return (
    <ChatHeaderMenu
      scope={scope}
      disabled={disabled}
      options={[
        {
          id: "settings",
          label: t("Settings"),
          Icon: Settings,
          onPress: onSettings,
        },
      ]}
      controls={(openMenu, trigger) => (
        <FloatingItem
          ref={trigger}
          accessibilityLabel={t("Crew options")}
          accessibilityState={{ expanded: false }}
          onPress={openMenu}
          disabled={disabled}
        >
          <MoreHorizontal size={23} color={colors.textPrimary} />
        </FloatingItem>
      )}
    />
  );
}
