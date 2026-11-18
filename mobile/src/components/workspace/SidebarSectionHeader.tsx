import { DynamicTypeText as Text } from "../DynamicTypeText";
import { useState } from "react";
import {
  ActivityIndicator,
  Pressable,
  StyleSheet,
  View,
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
import { sidebarSectionHeaderFitsInline } from "./workspaceLayout";

const ACTION_SIZE = 44;
const CHEVRON_SIZE = 16;

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
  const [rowWidth, setRowWidth] = useState(0);
  const [labelWidth, setLabelWidth] = useState(0);
  const [metaWidth, setMetaWidth] = useState(0);
  const Chevron = expanded ? ChevronDown : ChevronRight;
  const fitsInline = sidebarSectionHeaderFitsInline(
    rowWidth,
    labelWidth,
    metaWidth,
    CHEVRON_SIZE,
    (onRefresh ? 2 : 1) * ACTION_SIZE,
    styles.sectionRow.gap * 2 + styles.sectionRow.paddingRight,
  );
  return (
    <View
      testID="sidebar-section-viewport"
      onLayout={(event) => setRowWidth(event.nativeEvent.layout.width)}
    >
      <View
        testID="sidebar-section-heading"
        style={[styles.sectionHeading, !fitsInline && styles.stackedHeading]}
      >
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={t(section === "agents" ? expanded ? "Collapse agents" : "Expand agents" : section === "crews" ? expanded ? "Collapse crews" : "Expand crews" : expanded ? "Collapse {{section}}" : "Expand {{section}}", { section: label })}
          accessibilityState={{ expanded, disabled }}
          disabled={disabled}
          accessibilityValue={{ text: String(count) }}
          onPress={onPress}
          style={[styles.sectionRow, !fitsInline && styles.stackedRow]}
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
          <Text
            {...sidebarTextProps()}
            testID="sidebar-section-meta"
            numberOfLines={1}
            ellipsizeMode="tail"
            style={[styles.sectionCount, { color: colors.textMuted }]}
          >
            {count}
          </Text>
          <Chevron size={CHEVRON_SIZE} color={colors.textMuted} />
        </Pressable>
        <View style={styles.actions}>
          {onRefresh ? (
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={t(section === "agents" ? "Refresh agents" : section === "crews" ? "Refresh crews" : "Refresh {{section}}", {
                section: label,
              })}
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
        </View>
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
              setLabelWidth((event.nativeEvent.lines[0]?.width ?? 0) + styles.section.padding * 2)
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
  sectionHeading: { flexDirection: "row", alignItems: "center" },
  stackedHeading: { flexDirection: "column", alignItems: "stretch" },
  stackedRow: { flexGrow: 0, flexShrink: 0, flexWrap: "wrap" },
  actions: { flexDirection: "row", justifyContent: "flex-end" },
  createButton: {
    flexShrink: 0,
    width: ACTION_SIZE,
    height: ACTION_SIZE,
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
    maxWidth: "100%",
    flexShrink: 0,
    fontFamily: fonts.bodyRegular,
    fontSize: 11,
  },
  section: {
    ...sidebarSectionTitle,
    padding: 12,
    flexShrink: 0,
    maxWidth: "100%",
  },
});
