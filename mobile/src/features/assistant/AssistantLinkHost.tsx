import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  AccessibilityInfo,
  Modal,
  Pressable,
  ScrollView,
  View,
} from "react-native";
import { useNavigation } from "@react-navigation/native";
import * as WebBrowser from "expo-web-browser";
import { X } from "lucide-react-native";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { NYXID_FRONTEND_URL } from "../../lib/env";
import { useTheme } from "../../theme/ThemeContext";
import {
  openWorkspaceSheetAction,
  workspaceNavigateAction,
} from "../../app/navigationHierarchy";
import {
  ActionButton,
  Card,
  Label,
} from "../../components/workspace/WorkspaceUI";
import {
  workspaceCloseControlStyle,
  WORKSPACE_CLOSE_ICON_SIZE,
} from "../../components/workspace/workspaceCloseControl";
import { useGroupAvailability } from "../groups/useGroupRuntime";
import { AssistantHostedConnect } from "./AssistantHostedConnect";
import { AssistantLinkContext } from "./AssistantLinkContext";
import {
  assistantLinkTarget,
  type AssistantLinkTarget,
} from "./assistantLinkTarget";

/** Mounted per chat scope, preserving the composer while transient links open. */
export function AssistantLinkHost({
  scope,
  children,
}: {
  scope: string;
  children: ReactNode;
}) {
  return (
    <ScopedAssistantLinkHost scope={scope}>{children}</ScopedAssistantLinkHost>
  );
}
export function ScopedAssistantLinkHost({
  scope,
  children,
}: {
  scope: string;
  children: ReactNode;
}) {
  const navigation = useNavigation();
  const availability = useGroupAvailability();
  const { colors } = useTheme();
  const [opened, setOpened] = useState<{
    target: AssistantLinkTarget;
    scope: string;
    epoch: number;
  }>();
  const target =
    opened?.scope === scope && opened.epoch === getAuthEpoch()
      ? opened.target
      : undefined;
  const [busy, setBusy] = useState(false);
  const trigger = useRef<number | undefined>(undefined);
  const mounted = useRef(true);
  const available = useRef(availability.available);
  available.current = availability.available;
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const open = useCallback(
    (href: string, source?: number) => {
      const value = assistantLinkTarget(href, NYXID_FRONTEND_URL);
      if (!value) return false;
      if (!available.current()) return true;
      trigger.current = source;
      if (value.kind === "account" && value.panel !== "nyxbot") {
        if (value.panel === "billing")
          navigation.dispatch(
            openWorkspaceSheetAction("UsageCredits", { period: value.period }),
          );
        else if (
          ["profile", "security", "sessions"].includes(value.tab ?? "profile")
        )
          navigation.dispatch(openWorkspaceSheetAction("AccountSecurity"));
        else
          navigation.dispatch(
            workspaceNavigateAction("AccountSettings", {
              section:
                value.tab === "display"
                  ? "display"
                  : value.tab === "privacy"
                    ? "privacy"
                    : undefined,
            }),
          );
      } else setOpened({ target: value, scope, epoch: getAuthEpoch() });
      return true;
    },
    [navigation, scope],
  );
  useEffect(() => {
    setOpened(undefined);
    setBusy(false);
    trigger.current = undefined;
  }, [scope]);
  function close() {
    if (busy) return;
    setOpened(undefined);
  }
  function restoreFocus() {
    const source = trigger.current;
    if (source)
      requestAnimationFrame(() => {
        if (mounted.current && available.current())
          AccessibilityInfo.setAccessibilityFocus(source);
      });
  }
  return (
    <AssistantLinkContext.Provider value={open}>
      {children}
      <Modal
        transparent
        visible={!!target && availability.foreground && availability.focused}
        animationType="fade"
        onRequestClose={close}
        onDismiss={restoreFocus}
      >
        <View
          style={{
            flex: 1,
            backgroundColor: colors.sheetScrim,
            justifyContent: "center",
            padding: 20,
          }}
          accessibilityViewIsModal
        >
          <View
            style={{
              maxHeight: "90%",
              width: "100%",
              maxWidth: 520,
              alignSelf: "center",
              borderRadius: 16,
              backgroundColor: colors.card,
              borderColor: colors.border,
              borderWidth: 1,
              overflow: "hidden",
            }}
          >
            <View
              style={{
                flexDirection: "row",
                alignItems: "center",
                padding: 12,
                gap: 8,
              }}
            >
              <Label
                accessibilityRole="header"
                numberOfLines={1}
                maxFontSizeMultiplier={1.25}
                style={{ flex: 1, fontSize: 18 }}
              >
                {target?.kind === "connect"
                  ? "Connect a service"
                  : target?.kind === "channel"
                    ? "Connect a channel"
                    : "NyxBot settings"}
              </Label>
              <Pressable
                accessibilityRole="button"
                accessibilityLabel="Close"
                disabled={busy}
                onPress={close}
                style={workspaceCloseControlStyle(colors.cardSoft)}
              >
                <X
                  size={WORKSPACE_CLOSE_ICON_SIZE}
                  color={colors.textSecondary}
                />
              </Pressable>
            </View>
            <ScrollView
              keyboardShouldPersistTaps="handled"
              keyboardDismissMode="interactive"
              contentContainerStyle={{ padding: 16, gap: 12 }}
            >
              {target?.kind === "connect" ? (
                <AssistantHostedConnect
                  key={target.token}
                  token={target.token}
                  active={availability.active}
                  onBusy={setBusy}
                />
              ) : target ? (
                <Card>
                  <Label>
                    {target.kind === "channel"
                      ? `Continue setting up ${target.platform} on the web.`
                      : "View automations and manage agents here. Global limits and channel management are available on the web."}
                  </Label>
                  {target.kind === "account" ? (
                    <>
                      <ActionButton
                        label="Automations"
                        variant="secondary"
                        onPress={() => {
                          close();
                          navigation.dispatch(
                            workspaceNavigateAction("Automations"),
                          );
                        }}
                      />
                      <ActionButton
                        label="Agents"
                        variant="secondary"
                        onPress={() => {
                          close();
                          navigation.dispatch(
                            workspaceNavigateAction("Agents", {
                              tab: "agents",
                            }),
                          );
                        }}
                      />
                    </>
                  ) : null}
                  <ActionButton
                    label="Continue on the web"
                    onPress={() => {
                      if (available.current())
                        void WebBrowser.openBrowserAsync(target.href).catch(
                          () => undefined,
                        );
                    }}
                  />
                </Card>
              ) : null}
            </ScrollView>
          </View>
        </View>
      </Modal>
    </AssistantLinkContext.Provider>
  );
}
