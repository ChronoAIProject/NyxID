import { useEffect, useState } from "react";
import { Platform } from "react-native";
import { useIsFocused } from "@react-navigation/native";
import type { NativeStackScreenProps } from "@react-navigation/native-stack";
import type { RootStackParamList } from "../../app/navigationTypes";
import {
  ActionButton,
  Label,
  Notice,
  WorkspacePage,
} from "../../components/workspace/WorkspaceUI";
import { MachineDesktopScreen } from "./desktop/MachineDesktopScreen";
import { desktopRouteParams } from "./desktopEntry";
import { desktopPrivacyQueue, type DesktopPrivacy } from "./desktopPrivacy";

export function MachineDesktopRoute({
  route,
  navigation,
}: NativeStackScreenProps<RootStackParamList, "MachineDesktop">) {
  const focused = useIsFocused();
  const [ready, setReady] = useState(false);
  const [error, setError] = useState<string>();
  useEffect(() => {
    if (!focused) {
      setReady(false);
      return;
    }
    let mounted = true;
    let privacy: DesktopPrivacy | undefined;
    void desktopPrivacyQueue(async () => {
      // Loaded only for this route so an older development binary can still
      // open management pages while the required native module is rebuilt.
      privacy = await import("expo-screen-capture");
      await privacy.preventScreenCaptureAsync("machine-desktop");
      if (Platform.OS === "ios")
        await privacy.enableAppSwitcherProtectionAsync(1);
      if (mounted) setReady(true);
    }).catch(() => {
      if (mounted)
        setError(
          "Desktop privacy protection is unavailable. Install the current development build and try again.",
        );
    });
    return () => {
      mounted = false;
      void desktopPrivacyQueue(async () => {
        if (!privacy) return;
        // The viewer has unmounted and cleared its canvas before capture is allowed.
        if (Platform.OS === "ios")
          await privacy.disableAppSwitcherProtectionAsync();
        await privacy.allowScreenCaptureAsync("machine-desktop");
      }).catch(() => {});
    };
  }, [focused]);
  let params;
  try {
    params = desktopRouteParams(route.params);
  } catch {
    return (
      <WorkspacePage title="Live desktop">
        <Notice
          tone="error"
          message="This desktop link contains invalid or conflicting selectors."
        />
        <ActionButton label="Close desktop" onPress={navigation.goBack} />
      </WorkspacePage>
    );
  }
  if (!focused || !ready || error)
    return (
      <WorkspacePage title="Live desktop">
        {error ? (
          <Notice tone="error" message={error} />
        ) : (
          <Label muted>Protecting desktop privacy…</Label>
        )}
        <ActionButton label="Close desktop" onPress={navigation.goBack} />
      </WorkspacePage>
    );
  return (
    <MachineDesktopScreen
      route={{ params }}
      navigation={navigation}
      privacyProtected={ready}
    />
  );
}
