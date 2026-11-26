import { useAppLanguage } from "../i18n/LanguageProvider";
import { translateApp } from "../i18n/appTranslation";
import {
  WORKSPACE_CLOSE_ICON_SIZE,
  workspaceCloseControlStyle,
} from "../components/workspace/workspaceCloseControl";
import { WorkspaceInteractionSurface } from "../lib/accessibility/WorkspaceInteractionSurface";
import { useWorkspaceAccessibility } from "../lib/accessibility/workspaceAccessibility";
import { workspaceFocus } from "../lib/accessibility/workspaceFocus";
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import {
  useIsFocused,
  useNavigation,
  usePreventRemove,
  type NavigationAction,
} from "@react-navigation/native";
import type {
  NativeStackNavigationOptions,
  NativeStackNavigationProp,
} from "@react-navigation/native-stack";
import {
  Alert,
  BackHandler,
  findNodeHandle,
  Pressable,
  StyleSheet,
  View,
} from "react-native";
import { ChevronLeft, Menu, X } from "lucide-react-native";
import { FloatingButton } from "../components/FloatingControl";
import {
  WorkspaceColumnHeaderContext,
  type WorkspaceColumnHeaderControls,
} from "../components/workspace/workspaceColumnHeader";
import type { AgentIdentity } from "../components/agentAvatarModel";
import { useTheme } from "../theme/ThemeContext";
import {
  workspaceContextChatNavigationAction,
  type WorkspaceChatTarget,
} from "./workspaceChatNavigation";
import type { RootStackParamList } from "./AppNavigator";
import type {
  WorkspaceNavigationProp,
  AssistantParams,
} from "./navigationTypes";

export const WorkspaceSessionContext = createContext(true);
export const WorkspaceSheetContext = createContext<{
  registerDismissGuard?: (
    id: symbol,
    guard: () => Promise<boolean>,
  ) => () => void;
  dismissAuthorized?: boolean;
  rootKey: string;
  sheetKey: string;
  onClose: () => void;
  onOpenChat: (params?: AssistantParams) => void;
  onOpenGroup: (groupId: string) => void;
  /** Close the sheet onto an exact thread, as a sidebar row does. */
  onOpenTarget: (target: WorkspaceChatTarget) => void;
} | null>(null);

export function useWorkspaceSheet() {
  useAppLanguage();
  return useContext(WorkspaceSheetContext);
}

/** A chat opened from a workspace page keeps its caller and draft beneath it. */
export function useOpenWorkspaceChat() {
  useAppLanguage();
  const sheet = useContext(WorkspaceSheetContext);
  const navigation = useNavigation<WorkspaceNavigationProp>();
  return (params?: AssistantParams) => {
    if (sheet) sheet.onOpenChat(params);
    else {
      const action = workspaceContextChatNavigationAction(
        navigation.getState(),
        params ?? {},
      );
      if (action) navigation.dispatch(action);
    }
  };
}

export const WorkspaceMenuContext = createContext(() => {});
/**
 * Wide windows: Menu shows or hides the persistent sidebar instead of
 * opening a drawer, so its label names that toggle.
 */
export const WorkspaceSidebarToggleContext = createContext({
  persistent: false,
  collapsed: false,
});
export type WorkspaceNavigationGuardChange = (
  id: symbol,
  blocked: boolean,
  critical?: boolean,
) => void;
export const WorkspaceNavigationGuardContext =
  createContext<WorkspaceNavigationGuardChange>(() => {});

/** A cancelled leave attempt must also cancel its sidebar transition. */
export const WorkspaceNavigationCancelledContext = createContext(() => {});

export type WorkspaceEdgeBackChange = (id: symbol, back?: () => void) => void;
export const WorkspaceEdgeBackContext = createContext<WorkspaceEdgeBackChange>(
  () => {},
);

export function WorkspaceHeaderVisibility({
  children,
}: {
  children: React.ReactNode;
}) {
  useAppLanguage();
  const sheet = useContext(WorkspaceSheetContext);
  return (
    <WorkspaceInteractionSurface inSheet={Boolean(sheet)}>
      {children}
    </WorkspaceInteractionSurface>
  );
}

export type WorkspaceHeaderRole = "menu" | "back";

/** Menu/Back's spoken name, shared by the bar and the Duo's column. */
export function workspaceLeadingLabel(
  role: WorkspaceHeaderRole,
  sidebarToggle: { persistent: boolean; collapsed: boolean },
  t: (phrase: string) => string = translateApp,
) {
  return role === "back"
    ? t("Back")
    : sidebarToggle.persistent
      ? sidebarToggle.collapsed
        ? t("Show sidebar")
        : t("Hide sidebar")
      : t("Open navigation");
}

function WorkspaceHeaderButton({
  back = false,
  close = false,
  onPress,
  disabled,
  label,
}: {
  back?: boolean;
  close?: boolean;
  onPress: () => void;
  disabled: boolean;
  label?: string;
}) {
  const { t } = useAppLanguage();
  const { colors } = useTheme();
  const sheet = useContext(WorkspaceSheetContext);
  const sidebarToggle = useContext(WorkspaceSidebarToggleContext);
  // Every bar floats Menu/Back as the shared glass button: slab bars (narrow,
  // beside the persistent sidebar, the Duo) and sheet pages (Settings and its
  // account pages); only a sheet's own circular Close stays plain.
  const floating = !close;
  const control = useRef<View>(null);
  const focused = useIsFocused();
  const owner = useRef(Symbol("primary-header-control")).current;
  useEffect(() => () => workspaceFocus.release(owner), [owner]);
  const { sheetOpen, primaryKey, sidebarOpen } = useWorkspaceAccessibility();
  // A critical write can block leaving the page, but cannot trap an open menu.
  const controlDisabled = disabled && (back || close || !sidebarOpen);
  const hidden = !focused || (sheetOpen && !sheet);
  const primaryScope = useRef(primaryKey);
  if (focused && !sheet && !sheetOpen) primaryScope.current = primaryKey;
  useLayoutEffect(() => {
    if (sheet) return;
    workspaceFocus.register(
      owner,
      () => findNodeHandle(control.current),
      !hidden,
      primaryScope.current,
    );
  }, [owner, hidden, primaryKey, Boolean(sheet)]);
  const icon = close ? (
    <X color={colors.textPrimary} size={WORKSPACE_CLOSE_ICON_SIZE} />
  ) : back ? (
    <ChevronLeft color={colors.textPrimary} size={24} />
  ) : (
    <Menu color={colors.textPrimary} size={22} />
  );
  const controlProps = {
    ref: control,
    accessibilityElementsHidden: hidden,
    importantForAccessibility: hidden
      ? ("no-hide-descendants" as const)
      : ("auto" as const),
    accessible: !hidden,
    onPress: () => {
      if (!sheet) workspaceFocus.invoked(owner);
      onPress();
    },
    onLayout: () => {
      if (sheet && close)
        workspaceFocus.focusDrawerOnce(sheet.sheetKey, () =>
          findNodeHandle(control.current),
        );
    },
    accessibilityRole: "button" as const,
    accessibilityLabel:
      label ??
      (close
        ? t("Close drawer")
        : workspaceLeadingLabel(back ? "back" : "menu", sidebarToggle, t)),
    accessibilityState: { disabled: controlDisabled },
    disabled: controlDisabled,
  };
  return floating ? (
    <FloatingButton {...controlProps}>{icon}</FloatingButton>
  ) : (
    <Pressable
      {...controlProps}
      style={[
        styles.button,
        workspaceCloseControlStyle(colors.navActive),
        controlDisabled && styles.disabled,
      ]}
    >
      {icon}
    </Pressable>
  );
}

export function workspaceHeaderOptions({
  role,
  onBack,
  onOpenMenu,
  onClose,
  disabled = false,
  label,
}: {
  onOpenMenu: () => void;
  onClose?: () => void;
  disabled?: boolean;
  label?: string;
} & (
  | { role: "menu"; onBack?: () => void }
  | { role: "back"; onBack: () => void }
)): NativeStackNavigationOptions {
  const leftButton = () =>
    onClose && role === "menu" ? null : (
      <WorkspaceHeaderButton
        back={role === "back"}
        onPress={role === "back" ? onBack : onOpenMenu}
        disabled={disabled}
        label={role === "back" ? label : undefined}
      />
    );
  const closeButton = () =>
    onClose ? (
      <WorkspaceHeaderButton
        close
        onPress={onClose}
        disabled={disabled}
      />
    ) : null;
  return {
    headerShown: true,
    headerBackVisible: false,
    headerBackButtonMenuEnabled: false,
    headerLeft: leftButton,
    // Clear both right-side options when setOptions merges a previous header.
    headerRight: closeButton,
    unstable_headerLeftItems: () =>
      onClose && role === "menu"
        ? []
        : [
            {
              type: "custom",
              element: leftButton()!,
              hidesSharedBackground: true,
            },
          ],
    unstable_headerRightItems: () =>
      onClose
        ? [
            {
              type: "custom",
              element: closeButton()!,
              hidesSharedBackground: true,
            },
          ]
        : [],
  };
}

type ColumnControlInput = {
  label: string;
  onPress: () => void;
  disabled?: boolean;
};

/**
 * A chat's bar controls for the Duo, whose chats have no top bar in rail
 * mode: the status column shows them below the shortcuts (full screen: the
 * chat's own bar):
 * Menu (or Back, the same action as the bar's), the agent (the identity
 * pill's picker) and More. Registered while the chat is focused or is the
 * page beneath an open workspace sheet (never inside a sheet); handlers
 * always call the latest props.
 */
export function useWorkspaceColumnHeader({
  role,
  onBack,
  disabled = false,
  identity,
  more,
}: {
  role: WorkspaceHeaderRole;
  onBack: () => void;
  disabled?: boolean;
  identity: ColumnControlInput & {
    agent?: AgentIdentity;
    members?: readonly AgentIdentity[];
  };
  more: ColumnControlInput;
}) {
  const { t } = useAppLanguage();
  const register = useContext(WorkspaceColumnHeaderContext);
  const onOpenMenu = useContext(WorkspaceMenuContext);
  const sidebarToggle = useContext(WorkspaceSidebarToggleContext);
  const sheet = useContext(WorkspaceSheetContext);
  const focused = useIsFocused();
  // The same focus scope as the bar's Menu/Back: the workspace's, never a
  // sheet's opened over it.
  const { sheetOpen, primaryKey } = useWorkspaceAccessibility();
  const primaryScope = useRef(primaryKey);
  if (focused && !sheetOpen) primaryScope.current = primaryKey;
  const scope = primaryScope.current;
  // A workspace sheet (Settings, the agent picker, Recent chats) blurs the
  // chat beneath it, but the chat is still the page on screen: the Duo's
  // column keeps its controls in view beside the sheet. Only a chat that was
  // focused when the sheet opened keeps them.
  const focusedBeforeSheet = useRef(focused);
  if (!sheetOpen) focusedBeforeSheet.current = focused;
  const onScreen = focused || (sheetOpen && focusedBeforeSheet.current);
  const owner = useRef(Symbol("workspace-column-header")).current;
  const focusOwner = useRef(Symbol("column-header-control")).current;
  const latest = useRef({ onBack, onOpenMenu, identity, more });
  latest.current = { onBack, onOpenMenu, identity, more };
  const leadingLabel = workspaceLeadingLabel(role, sidebarToggle, t);
  // Re-register only when what the column shows changes.
  const shown = JSON.stringify([
    identity.label,
    Boolean(identity.disabled),
    identity.agent ?? null,
    identity.members ?? null,
    more.label,
    Boolean(more.disabled),
  ]);
  useLayoutEffect(() => {
    if (!onScreen || sheet) {
      register(owner, null);
      return;
    }
    const current = latest.current;
    const controls: WorkspaceColumnHeaderControls = {
      leading: {
        role,
        label: leadingLabel,
        disabled,
        onPress: () => {
          workspaceFocus.invoked(focusOwner);
          if (role === "back") latest.current.onBack();
          else latest.current.onOpenMenu();
        },
        registerFocus: (target, active) =>
          workspaceFocus.register(focusOwner, target, active, scope),
      },
      identity: {
        label: current.identity.label,
        disabled: current.identity.disabled,
        agent: current.identity.agent,
        members: current.identity.members,
        onPress: () => latest.current.identity.onPress(),
      },
      more: {
        label: current.more.label,
        disabled: current.more.disabled,
        onPress: () => latest.current.more.onPress(),
      },
    };
    register(owner, controls);
  }, [
    onScreen,
    Boolean(sheet),
    register,
    owner,
    focusOwner,
    role,
    leadingLabel,
    disabled,
    shown,
    scope,
  ]);
  useEffect(
    () => () => {
      register(owner, null);
      workspaceFocus.release(focusOwner);
    },
    [register, owner, focusOwner],
  );
}

/** Keep a screen's local detail/form history ahead of the native route history. */
export function useWorkspaceBack({
  active,
  onBack,
  defaultRole = "menu",
  disabled = false,
  label = translateApp("Back"),
  options,
  discardOnClose,
}: {
  active: boolean;
  onBack: () => void;
  defaultRole?: WorkspaceHeaderRole;
  disabled?: boolean;
  label?: string;
  /** Screen-specific title/actions must be applied with the navigation role. */
  options?: NativeStackNavigationOptions;
  /** Dirty forms guard navigation away as well as Settings surface dismissal. */
  discardOnClose?: { title: string; message: string };
}) {
  const { t } = useAppLanguage();
  const navigation =
    useNavigation<NativeStackNavigationProp<RootStackParamList>>();
  const focused = useIsFocused();
  const sessionActive = useContext(WorkspaceSessionContext);
  const onOpenMenu = useContext(WorkspaceMenuContext);
  const sheet = useContext(WorkspaceSheetContext);
  const onNavigationGuardChange = useContext(WorkspaceNavigationGuardContext);
  const onEdgeBackChange = useContext(WorkspaceEdgeBackContext);
  const onNavigationCancelled = useContext(WorkspaceNavigationCancelledContext);
  const guardId = useRef(Symbol("workspace-navigation-guard"));
  const current = useRef({ onBack, disabled, focused, sessionActive });
  const confirmationPending = useRef(false);
  const mounted = useRef(true);
  const [allowedRemoval, setAllowedRemoval] = useState<NavigationAction>();
  const allowedFocus = useRef(true);
  current.current = { onBack, disabled, focused, sessionActive };
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const localBack = useCallback(() => {
    if (!current.current.disabled) current.current.onBack();
  }, []);

  useLayoutEffect(() => {
    if (!focused || (!active && !discardOnClose) || sheet) return;
    const id = guardId.current;
    onEdgeBackChange(id, localBack);
    return () => onEdgeBackChange(id);
  }, [
    focused,
    active,
    Boolean(discardOnClose),
    Boolean(sheet),
    localBack,
    onEdgeBackChange,
  ]);

  useLayoutEffect(() => {
    if (!focused) return;
    navigation.setOptions({
      ...workspaceHeaderOptions({
        role: active ? "back" : defaultRole,
        onOpenMenu,
        onClose: sheet?.onClose,
        onBack: active ? localBack : navigation.goBack,
        disabled,
        label: active ? label : translateApp("Back"),
      }),
      gestureEnabled:
        !disabled && (Boolean(sheet) || (!active && defaultRole === "back")),
      ...options,
    });
  }, [
    active,
    defaultRole,
    disabled,
    focused,
    label,
    localBack,
    navigation,
    onOpenMenu,
    options,
    sheet,
    t,
  ]);

  useEffect(() => {
    if (
      !sessionActive ||
      !sheet?.registerDismissGuard ||
      (!discardOnClose && !(focused && disabled))
    )
      return;
    return sheet.registerDismissGuard(guardId.current, async () => {
      if (current.current.disabled) return false;
      if (!discardOnClose) return true;
      return new Promise<boolean>((resolve) => {
        Alert.alert(
          discardOnClose.title,
          discardOnClose.message,
          [
            {
              text: t("Keep editing"),
              style: "cancel",
              onPress: () => resolve(false),
            },
            {
              text: t("Discard"),
              style: "destructive",
              onPress: () =>
                resolve(mounted.current && !current.current.disabled),
            },
          ],
          { cancelable: true, onDismiss: () => resolve(false) },
        );
      });
    });
  }, [
    sessionActive,
    sheet?.registerDismissGuard,
    Boolean(discardOnClose),
    discardOnClose?.title,
    discardOnClose?.message,
    t,
    disabled,
    focused,
  ]);

  usePreventRemove(
    sessionActive &&
      !sheet?.dismissAuthorized &&
      ((focused && (active || disabled)) || Boolean(discardOnClose)) &&
      (!allowedRemoval || disabled),
    ({ data }) => {
      const outerDismiss = sheet
        ? data.action.target === sheet.rootKey ||
          data.action.source === sheet.sheetKey
        : !active ||
          ["RESET", "NAVIGATE", "REPLACE", "POP_TO"].includes(data.action.type);
      if (!outerDismiss) {
        localBack();
        return;
      }
      if (current.current.disabled) return;
      if (confirmationPending.current) return;
      const focusWhenRequested = current.current.focused;
      const proceed = () => {
        if (
          mounted.current &&
          current.current.focused === focusWhenRequested &&
          !current.current.disabled &&
          current.current.sessionActive
        ) {
          allowedFocus.current = focusWhenRequested;
          setAllowedRemoval(data.action);
        }
      };
      if (discardOnClose) {
        confirmationPending.current = true;
        let accepted = false;
        const resetConfirmation = () => {
          confirmationPending.current = false;
        };
        const cancel = () => {
          if (accepted) return;
          resetConfirmation();
          onNavigationCancelled();
        };
        Alert.alert(
          discardOnClose.title,
          discardOnClose.message,
          [
            { text: t("Keep editing"), style: "cancel", onPress: cancel },
            {
              text: t("Discard"),
              style: "destructive",
              onPress: () => {
                accepted = true;
                resetConfirmation();
                proceed();
              },
            },
          ],
          { cancelable: true, onDismiss: cancel },
        );
      } else proceed();
    },
  );

  // Replay the exact parent action after React has removed this route's guard.
  // Replacing it with a child goBack would only move one level inside the drawer.
  useEffect(() => {
    if (!allowedRemoval) return;
    if (
      mounted.current &&
      current.current.focused === allowedFocus.current &&
      !current.current.disabled
    )
      navigation.dispatch(allowedRemoval);
    setAllowedRemoval(undefined);
  }, [allowedRemoval, navigation]);

  // Defer external approval navigation while a critical write forbids leaving.
  // Local detail Back handling remains independent from notification delivery.
  useLayoutEffect(() => {
    if (!sessionActive || (!(focused && disabled) && !discardOnClose)) return;
    const id = guardId.current;
    onNavigationGuardChange(id, true, focused && disabled);
    return () => onNavigationGuardChange(id, false);
  }, [
    sessionActive,
    disabled,
    Boolean(discardOnClose),
    focused,
    onNavigationGuardChange,
  ]);

  useEffect(() => {
    if (!focused || (!active && !disabled)) return;
    const subscription = BackHandler.addEventListener(
      "hardwareBackPress",
      () => {
        localBack();
        return true;
      },
    );
    return () => subscription.remove();
  }, [active, disabled, focused, localBack]);
}

const styles = StyleSheet.create({
  button: {
    minWidth: 44,
    width: 44,
    height: 44,
    justifyContent: "center",
    alignItems: "center",
  },
  disabled: { opacity: 0.4 },
});
