import { Text } from "react-native";
import { useContext } from "react";
import { WorkspaceCornerSensorContext } from "./WorkspaceLayoutContext";
import { useState } from "react";
import {
  ActivityIndicator,
  Pressable,
  StyleSheet,
  View
} from "react-native";
import {
  ChevronDown,
  ChevronRight,
  Plus,
  RefreshCw,
} from "lucide-react-native";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { useTheme } from "../../theme/ThemeContext";
import { fonts } from "../../theme/designTokens";
import { sidebarSectionTitle } from "./workspaceSidebarStyles";
import { sidebarTextProps } from "./sidebarText";
import { sidebarSectionHeaderMetaVisible } from "./workspaceLayout";

export function SidebarSectionHeader({
  label,
  section,
  count,
  expanded,
  onPress,
  disabled = false,
  createLabel,
  onCreate,
  createDisabled,
  onRefresh,
  refreshing = false,
  refreshDisabled = false,
}: {
  label: string;
  section?: "agents" | "crews";
  count: number | string;
  expanded: boolean;
  onPress: () => void;
  /** A critical write is blocking navigation: the menu stays readable but inert. */
  disabled?: boolean;
  createLabel: string;
  onCreate: () => void;
  createDisabled: boolean;
  onRefresh?: () => void;
  refreshing?: boolean;
  refreshDisabled?: boolean;
}) {
  const { t } = useAppLanguage();
  const { colors } = useTheme();
  const cornerWindow = useContext(WorkspaceCornerSensorContext);
  const [rowWidth, setRowWidth] = useState(0);
  const [labelWidth, setLabelWidth] = useState(0);
  const [metaWidth, setMetaWidth] = useState(0);
  const [chevronWidth, setChevronWidth] = useState(0);
  const [refreshWidth, setRefreshWidth] = useState(0);
  const [createWidth, setCreateWidth] = useState(0);
  const Chevron = expanded ? ChevronDown : ChevronRight;
  const showMeta = sidebarSectionHeaderMetaVisible(
    rowWidth,
    labelWidth,
    metaWidth,
    chevronWidth,
    refreshWidth + createWidth,
    styles.sectionRow.gap * 2 + styles.sectionRow.paddingRight,
  );
  if (!cornerWindow) {
    return (
      <View style={baseStyles.sectionHeading}>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={t(
            section === "agents"
              ? expanded
                ? "Collapse agents"
                : "Expand agents"
              : section === "crews"
                ? expanded
                  ? "Collapse crews"
                  : "Expand crews"
                : expanded
                  ? "Collapse {{section}}"
                  : "Expand {{section}}",
            { section: label },
          )}
          accessibilityState={{ expanded, disabled }}
          disabled={disabled}
          accessibilityValue={{ text: String(count) }}
          onPress={onPress}
          style={baseStyles.sectionRow}
        >
          <Text
            accessibilityRole="header"
            style={[
              baseStyles.section,
              { color: disabled ? colors.textMuted : colors.textPrimary },
            ]}
          >
            {label}
          </Text>
          <Text style={[baseStyles.sectionCount, { color: colors.textMuted }]}>
            {count}
          </Text>
          <Chevron size={16} color={colors.textMuted} />
        </Pressable>
        {onRefresh ? (
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={t(
              section === "agents"
                ? "Refresh agents"
                : section === "crews"
                  ? "Refresh crews"
                  : "Refresh {{section}}",
              { section: label },
            )}
            accessibilityState={{
              disabled: refreshDisabled || refreshing,
              busy: refreshing,
            }}
            disabled={refreshDisabled || refreshing}
            onPress={onRefresh}
            style={[
              baseStyles.createButton,
              { opacity: refreshDisabled ? 0.4 : 1 },
            ]}
          >
            {refreshing ? (
              <ActivityIndicator size="small" color={colors.textSecondary} />
            ) : (
              <RefreshCw size={18} color={colors.textSecondary} />
            )}
          </Pressable>
        ) : null}
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={createLabel}
          accessibilityState={{ disabled: createDisabled }}
          disabled={createDisabled}
          onPress={onCreate}
          style={baseStyles.createButton}
        >
          <Plus
            size={18}
            color={createDisabled ? colors.textMuted : colors.textSecondary}
          />
        </Pressable>
      </View>
    );
  }
  return (
    <View
      testID="sidebar-section-viewport"
      onLayout={(event) => setRowWidth(event.nativeEvent.layout.width)}
    >
      <View style={styles.sectionHeading}>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={t(section === "agents" ? expanded ? "Collapse agents" : "Expand agents" : section === "crews" ? expanded ? "Collapse crews" : "Expand crews" : expanded ? "Collapse {{section}}" : "Expand {{section}}", { section: label },)}
          accessibilityState={{ expanded, disabled }}
          disabled={disabled}
          accessibilityValue={{ text: String(count) }}
          onPress={onPress}
          style={styles.sectionRow}
        >
          <Text
            {...sidebarTextProps()}
            accessibilityRole="header"
            numberOfLines={1}
            ellipsizeMode="tail"
            testID="sidebar-section-label"
            style={[
              styles.section,
              { color: disabled ? colors.textMuted : colors.textPrimary },
            ]}
          >
            {label}
          </Text>
          <View
            accessible={false}
            testID="sidebar-section-meta"
            accessibilityElementsHidden={!showMeta}
            importantForAccessibility={
              showMeta ? "auto" : "no-hide-descendants"
            }
            style={{ width: showMeta ? metaWidth : 0, overflow: "hidden" }}
          >
            <Text
              {...sidebarTextProps()}
              numberOfLines={1}
              ellipsizeMode="tail"
              style={[
                styles.sectionCount,
                { color: colors.textMuted, width: metaWidth || undefined },
              ]}
            >
              {count}
            </Text>
          </View>
          <View
            testID="sidebar-section-chevron"
            onLayout={(event) =>
              setChevronWidth(event.nativeEvent.layout.width)
            }
          >
            <Chevron size={16} color={colors.textMuted} />
          </View>
        </Pressable>
        {onRefresh ? (
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={t(section === "agents" ? "Refresh agents" : section === "crews" ? "Refresh crews" : "Refresh {{section}}", {
              section: label,
            },)}
            onLayout={(event) =>
              setRefreshWidth(event.nativeEvent.layout.width)
            }
            accessibilityState={{
              disabled: refreshDisabled || refreshing,
              busy: refreshing,
            }}
            disabled={refreshDisabled || refreshing}
            onPress={onRefresh}
            style={[
              styles.createButton,
              { opacity: refreshDisabled ? 0.4 : 1 },
            ]}
          >
            {refreshing ? (
              <ActivityIndicator size="small" color={colors.textSecondary} />
            ) : (
              <RefreshCw size={18} color={colors.textSecondary} />
            )}
          </Pressable>
        ) : null}
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={createLabel}
          onLayout={(event) => setCreateWidth(event.nativeEvent.layout.width)}
          accessibilityState={{ disabled: createDisabled }}
          disabled={createDisabled}
          onPress={onCreate}
          style={styles.createButton}
        >
          <Plus
            size={18}
            color={createDisabled ? colors.textMuted : colors.textSecondary}
          />
        </Pressable>
        <View
          pointerEvents="none"
          accessible={false}
          accessibilityElementsHidden
          importantForAccessibility="no-hide-descendants"
          style={{
            position: "absolute",
            width: 10000,
            height: 0,
            overflow: "hidden",
          }}
        >
          <Text
            {...sidebarTextProps()}
            numberOfLines={1}
            testID="sidebar-section-label-measure"
            onTextLayout={(event) =>
              setLabelWidth((event.nativeEvent.lines[0]?.width ?? 0) + styles.section.padding * 2,)
            }
            style={styles.section}
          >
            {label}
          </Text>
          <Text
            {...sidebarTextProps()}
            numberOfLines={1}
            ellipsizeMode="tail"
            testID="sidebar-section-meta-measure"
            onTextLayout={(event) =>
              setMetaWidth(event.nativeEvent.lines[0]?.width ?? 0)
            }
            style={styles.sectionCount}
          >
            {count}
          </Text>
        </View>
      </View>
    </View>
  );
}
const styles = StyleSheet.create({
  sectionHeading: { flex: 1, flexDirection: "row", alignItems: "center" },
  createButton: {
    flexShrink: 0,
    width: 44,
    height: 44,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: 8,
  },
  sectionRow: {
    flexGrow: 1,
    flexShrink: 1,
    minWidth: 0,
    flexDirection: "row",
    alignItems: "center",
    minHeight: 44,
    paddingRight: 10,
    gap: 8,
  },
  sectionCount: {
    flexShrink: 0,
    fontFamily: fonts.bodyRegular,
    fontSize: 11,
  },
  section: {
    ...sidebarSectionTitle,
    padding: 12,
    flexGrow: 0,
    flexShrink: 1,
    minWidth: 0,
  },
});

const baseStyles = StyleSheet.create({
  sectionHeading: { flexDirection: "row", alignItems: "center" },
  createButton: {
    width: 44,
    height: 44,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: 8,
  },
  sectionRow: {
    flex: 1,
    flexDirection: "row",
    alignItems: "center",
    minHeight: 44,
    paddingRight: 10,
    gap: 8,
  },
  sectionCount: { fontFamily: fonts.bodyRegular, fontSize: 11 },
  section: {
    ...sidebarSectionTitle,
    padding: 12,
    flex: 1,
  },
});
