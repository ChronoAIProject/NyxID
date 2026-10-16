import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useLayoutEffect,
  useRef,
} from "react";
import {
  useIsFocused,
  useNavigation,
  usePreventRemove,
} from "@react-navigation/native";
import type {
  NativeStackNavigationOptions,
  NativeStackNavigationProp,
} from "@react-navigation/native-stack";
import { BackHandler, Pressable, StyleSheet } from "react-native";
import { ChevronLeft, Menu } from "lucide-react-native";
import { useTheme } from "../theme/ThemeContext";
import type { RootStackParamList } from "./AppNavigator";

export const WorkspaceMenuContext = createContext(() => {});
export type WorkspaceNavigationGuardChange = (
  id: symbol,
  blocked: boolean,
) => void;
export const WorkspaceNavigationGuardContext =
  createContext<WorkspaceNavigationGuardChange>(() => {});

export type WorkspaceHeaderRole = "menu" | "back";

function WorkspaceHeaderButton({
  back = false,
  onPress,
  disabled,
  label,
}: {
  back?: boolean;
  onPress: () => void;
  disabled: boolean;
  label: string;
}) {
  const { colors } = useTheme();
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled }}
      disabled={disabled}
      style={[styles.button, disabled && styles.disabled]}
    >
      {back ? (
        <ChevronLeft color={colors.textPrimary} size={24} />
      ) : (
        <Menu color={colors.textPrimary} size={22} />
      )}
    </Pressable>
  );
}

export function workspaceHeaderOptions({
  role,
  onBack,
  onOpenMenu,
  disabled = false,
  label = "Back",
}: {
  onOpenMenu: () => void;
  disabled?: boolean;
  label?: string;
} & (
  | { role: "menu"; onBack?: () => void }
  | { role: "back"; onBack: () => void }
)): NativeStackNavigationOptions {
  const leftButton = () => (
    <WorkspaceHeaderButton
      back={role === "back"}
      onPress={role === "back" ? onBack : onOpenMenu}
      disabled={disabled}
      label={role === "back" ? label : "Open navigation"}
    />
  );
  return {
    headerShown: true,
    headerBackVisible: false,
    headerBackButtonMenuEnabled: false,
    headerLeft: leftButton,
    // Clear both right-side options when setOptions merges a previous header.
    headerRight: () => null,
    unstable_headerLeftItems: () => [
      {
        type: "custom",
        element: leftButton(),
        hidesSharedBackground: true,
      },
    ],
    unstable_headerRightItems: () => [],
  };
}

/** Keep a screen's local detail/form history ahead of the native route history. */
export function useWorkspaceBack({
  active,
  onBack,
  defaultRole = "menu",
  disabled = false,
  label = "Back",
}: {
  active: boolean;
  onBack: () => void;
  defaultRole?: WorkspaceHeaderRole;
  disabled?: boolean;
  label?: string;
}) {
  const navigation =
    useNavigation<NativeStackNavigationProp<RootStackParamList>>();
  const focused = useIsFocused();
  const onOpenMenu = useContext(WorkspaceMenuContext);
  const onNavigationGuardChange = useContext(WorkspaceNavigationGuardContext);
  const guardId = useRef(Symbol("workspace-navigation-guard"));
  const current = useRef({ onBack, disabled });
  current.current = { onBack, disabled };
  const localBack = useCallback(() => {
    if (!current.current.disabled) current.current.onBack();
  }, []);

  useLayoutEffect(() => {
    if (!focused) return;
    navigation.setOptions({
      ...workspaceHeaderOptions({
        role: active ? "back" : defaultRole,
        onOpenMenu,
        onBack: active ? localBack : navigation.goBack,
        disabled,
        label: active ? label : "Back",
      }),
      gestureEnabled: !active && !disabled,
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
  ]);

  usePreventRemove(focused && (active || disabled), localBack);

  // Defer external approval navigation while a critical write forbids leaving.
  // Local detail Back handling remains independent from notification delivery.
  useLayoutEffect(() => {
    if (!focused || !disabled) return;
    const id = guardId.current;
    onNavigationGuardChange(id, true);
    return () => onNavigationGuardChange(id, false);
  }, [disabled, focused, onNavigationGuardChange]);

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
    height: 44,
    justifyContent: "center",
    alignItems: "center",
  },
  disabled: { opacity: 0.4 },
});
