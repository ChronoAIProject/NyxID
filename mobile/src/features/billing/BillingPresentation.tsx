import { Pressable, StyleSheet, View, useWindowDimensions } from "react-native";
import { ChevronDown, ChevronRight } from "lucide-react-native";
import { Card, Label } from "../../components/workspace/WorkspaceUI";
import { useTheme } from "../../theme/ThemeContext";
import { radius, spacing, typeScale } from "../../theme/designTokens";
import { formatCredits, formatExactCredits } from "./credits";
import type { BillingAllowance, BillingGrant, BillingWallet, serviceUsage } from "./billingContracts";
import { balanceRows } from "./balanceSummary";

export function BillingBadge({ label }: { label: string }) {
  const { colors } = useTheme();
  return <View style={[styles.badge, { backgroundColor: colors.cardSoft, borderColor: colors.border }]}>
    <Label style={typeScale.badge}>{label}</Label>
  </View>;
}

export function BillingFigure({ label, value, exactValue, unit = "credits" }: { label: string; value: string | null; exactValue?: string | null; unit?: string }) {
  const { colors } = useTheme();
  const { fontScale } = useWindowDimensions();
  const stacked = fontScale >= 1.4;
  const money = unit === "credits" && value !== null;
  return <View accessible accessibilityLabel={`${label}: ${value === null ? "unavailable" : `${exactValue ?? value}${unit ? ` ${unit}` : ""}`}`} style={[styles.figure, stacked && styles.figureStacked]}>
    <Label muted style={stacked ? styles.fullWidth : styles.figureLabel}>{label}</Label>
    <Label key={`amount-${fontScale}`} mono={money} numberOfLines={money ? 1 : undefined} adjustsFontSizeToFit={money} minimumFontScale={money ? 0.25 : undefined}
      style={[styles.amount, money && styles.tabular, { color: value?.startsWith("-") || value?.startsWith(">-") ? colors.danger : colors.textPrimary }, stacked && styles.stackedAmount]}>
      {value ?? "Unavailable"}{value === null || !unit ? "" : ` ${unit}`}
    </Label>
  </View>;
}

export function BillingWalletCard({ wallet, grants, allowances, now }: { wallet: BillingWallet; grants: BillingGrant[]; allowances: BillingAllowance[]; now: number }) {
  const { colors } = useTheme();
  const { fontScale } = useWindowDimensions();
  return <Card>
    <Label muted>Available credits</Label>
    <Label key={`balance-${fontScale}`} mono accessibilityLabel={`Available credits: ${formatCredits(wallet.available)}`}
      numberOfLines={1} adjustsFontSizeToFit minimumFontScale={0.25}
      style={[styles.balance, { color: wallet.available?.startsWith("-") ? colors.danger : colors.textPrimary }]}>
      {wallet.available === null ? "Unavailable" : formatExactCredits(wallet.available)}
    </Label>
    {balanceRows(wallet, grants, allowances, now).map((row) => <BillingFigure key={row.key} label={row.label} value={row.value} exactValue={row.exactValue} unit={row.unit} />)}
  </Card>;
}

export function BillingServiceCard({ service }: { service: ReturnType<typeof serviceUsage>[number] }) {
  return <Card>
    <BillingFigure label={service.name} value={service.cost === null ? null : formatExactCredits(service.cost)} exactValue={service.cost === null ? null : formatCredits(service.cost)} />
    {service.meteringOnly && <BillingBadge label="Metering only" />}
  </Card>;
}

export function BillingHistoryRow({ expanded, onPress }: { expanded: boolean; onPress: () => void }) {
  const { colors } = useTheme();
  const Chevron = expanded ? ChevronDown : ChevronRight;
  return <Pressable accessibilityRole="button" accessibilityLabel="Usage history" accessibilityState={{ expanded }} onPress={onPress}
    style={({ pressed }) => [styles.history, { borderColor: colors.border, backgroundColor: pressed ? colors.cardSoft : colors.card }]}>
    <Label style={styles.figureLabel}>Usage history</Label>
    <Chevron size={18} color={colors.textMuted} accessible={false} />
  </Pressable>;
}

const styles = StyleSheet.create({
  badge: { alignSelf: "flex-start", borderWidth: 1, borderRadius: radius.sm, paddingHorizontal: spacing.sm, paddingVertical: spacing.xxs },
  figure: { flexDirection: "row", alignItems: "flex-start", gap: spacing.lg },
  figureStacked: { flexDirection: "column", gap: spacing.xs },
  fullWidth: { flex: 0, width: "100%" },
  stackedAmount: { flex: 0, width: "100%", textAlign: "left", alignSelf: "flex-start" },
  figureLabel: { flex: 1 },
  amount: { flex: 1.4, textAlign: "right" },
  tabular: { fontVariant: ["tabular-nums"] },
  balance: { fontSize: 32, lineHeight: 40, textAlign: "right", fontVariant: ["tabular-nums"] },
  history: { minHeight: 48, borderWidth: 1, borderRadius: radius.lg, padding: spacing.lg, flexDirection: "row", alignItems: "center", gap: spacing.sm },
});
