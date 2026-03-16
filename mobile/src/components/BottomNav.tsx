import { Pressable, StyleSheet, Text, View } from "react-native";
import { mobileTheme } from "../theme/mobileTheme";
import { radius, spacing, typeScale } from "../theme/designTokens";

export type BottomNavTab = "dashboard" | "inbox" | "approvals" | "account";

type BottomNavProps = {
  active: BottomNavTab;
  onTabPress: (tab: BottomNavTab) => void;
};

const tabs: Array<{ key: BottomNavTab; label: string }> = [
  { key: "dashboard", label: "Dashboard" },
  { key: "inbox", label: "Challenges" },
  { key: "approvals", label: "Approvals" },
  { key: "account", label: "Account" },
];

export function BottomNav({ active, onTabPress }: BottomNavProps) {
  return (
    <View style={styles.wrap}>
      {tabs.map((tab) => (
        <Pressable
          key={tab.key}
          onPress={() => onTabPress(tab.key)}
          style={[styles.item, active === tab.key && styles.itemActive]}
        >
          <Text style={[styles.text, active === tab.key && styles.textActive]}>{tab.label}</Text>
        </Pressable>
      ))}
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: {
    backgroundColor: mobileTheme.card,
    borderRadius: radius.xl,
    borderWidth: 1,
    borderColor: mobileTheme.border,
    padding: spacing.xs + spacing.xxs,
    flexDirection: "row",
    gap: spacing.xs + spacing.xxs,
  },
  item: {
    flex: 1,
    paddingVertical: spacing.md,
    borderRadius: radius.md,
    alignItems: "center",
    justifyContent: "center",
    backgroundColor: "transparent",
  },
  itemActive: {
    backgroundColor: "#232136",
  },
  text: {
    color: mobileTheme.textMuted,
    ...typeScale.caption,
    fontWeight: "600",
  },
  textActive: {
    color: mobileTheme.textPrimary,
  },
});
