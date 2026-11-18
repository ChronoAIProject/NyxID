import { PropsWithChildren, useContext, useMemo } from "react";
import { SafeAreaView as NativeSafeAreaView, StyleSheet } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { WorkspaceCornerSensorContext } from "./workspace/WorkspaceLayoutContext";
import { workspaceBodyEdges } from "./workspace/workspaceLayout";
import { RailInsetView } from "./workspace/WorkspaceSlabMotion";
import { useTheme } from "../theme/ThemeContext";
import type { ThemeColors } from "../theme/mobileTheme";

export function ScreenContainer({ children }: PropsWithChildren) {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);
  const sensorStrip = useContext(WorkspaceCornerSensorContext);
  // Beside a corner status column bodies run to the right edge (below the
  // header band); elsewhere the native safe area applies.
  const reclaim = sensorStrip;
  const Surface = reclaim ? SafeAreaView : NativeSafeAreaView;

  return (
    <Surface
      style={styles.safe}
      {...(reclaim ? { edges: workspaceBodyEdges(true) } : {})}>
      {/* Readable content keeps clear of the Duo's rail in rail mode
          and runs to the margin in full screen (animated). */}
      <RailInsetView style={styles.content}>{children}</RailInsetView>
    </Surface>
  );
}

const createStyles = (c: ThemeColors) =>
  StyleSheet.create({
    safe: { flex: 1, backgroundColor: c.bg },
    content: { flex: 1 },
  });
