import { SettingsGroup, SettingsRow } from "../../components/settings/SettingsList";
import { formatCredits, formatExactCredits } from "./credits";
import type { BillingAllowance, BillingGrant, BillingWallet, serviceUsage } from "./billingContracts";
import { balanceRows } from "./balanceSummary";

/** Balance and included benefits as iOS groups: label on the left, value on the right. */
export function BillingWalletCard({ wallet, grants, allowances, now }: { wallet: BillingWallet; grants: BillingGrant[]; allowances: BillingAllowance[]; now: number }) {
  const rows = balanceRows(wallet, grants, allowances, now);
  const credits = rows.filter((row) => !row.group);
  const groups = new Map<string, typeof rows>();
  for (const row of rows) if (row.group) groups.set(row.group, [...(groups.get(row.group) ?? []), row]);
  return <>
    <SettingsGroup title="Balance" inset={false}>
      <SettingsRow label="Available credits" value={wallet.available === null ? "Unavailable" : `${formatExactCredits(wallet.available)} credits`}
        valueTone={wallet.available?.startsWith("-") ? "danger" : "default"}
        accessibilityLabel={`Available credits: ${formatCredits(wallet.available)}`} />
      {credits.map((row) => <SettingsRow key={row.key} label={row.label} value={`${row.value}${row.unit === "" ? "" : ` ${row.unit ?? "credits"}`}`}
        valueTone={row.value.startsWith("-") ? "danger" : "default"}
        accessibilityLabel={`${row.label}: ${row.exactValue ?? row.value}${row.unit === "" ? "" : ` ${row.unit ?? "credits"}`}`} />)}
    </SettingsGroup>
    {[...groups].map(([group, items]) => <SettingsGroup key={group} title={`Included · ${group}`} inset={false}>
      {items.map((row) => <SettingsRow key={row.key} label={row.item ?? row.label} value={row.left ?? row.value} />)}
    </SettingsGroup>)}
  </>;
}

export function BillingServiceCard({ service }: { service: ReturnType<typeof serviceUsage>[number] }) {
  return <SettingsRow label={service.name} detail={service.meteringOnly ? "Metering only" : undefined}
    value={service.cost === null ? "Unavailable" : `${formatExactCredits(service.cost)} credits`}
    accessibilityLabel={`${service.name}: ${service.cost === null ? "unavailable" : `${formatCredits(service.cost)} credits`}`} />;
}

export function BillingHistoryRow({ expanded, onPress }: { expanded: boolean; onPress: () => void }) {
  return <SettingsGroup inset={false}>
    <SettingsRow label="Usage history" value={expanded ? "Hide" : undefined} accessibilityState={{ expanded }} onPress={onPress} />
  </SettingsGroup>;
}
