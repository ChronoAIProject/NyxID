import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp } from "../../i18n/appTranslation";
import { SettingsGroup, SettingsRow } from "../../components/settings/SettingsList";
import { formatCredits, formatExactCredits } from "./credits";
import type { BillingAllowance, BillingGrant, BillingWallet, serviceUsage } from "./billingContracts";
import { balanceRows } from "./balanceSummary";

/** Balance and included benefits as iOS groups: label on the left, value on the right. */
export function BillingWalletCard({ wallet, grants, allowances, now }: { wallet: BillingWallet; grants: BillingGrant[]; allowances: BillingAllowance[]; now: number;
}) {
  useAppLanguage();
  const rows = balanceRows(wallet, grants, allowances, now);
  const credits = rows.filter((row) => !row.group);
  const groups = new Map<string, typeof rows>();
  for (const row of rows) if (row.group) groups.set(row.group, [...(groups.get(row.group) ?? []), row]);
  return (
    <>
    <SettingsGroup title={translateApp("Balance")} inset={false}>
      <SettingsRow label={translateApp("Available credits")}
          value={wallet.available === null ? translateApp("Unavailable")
              : translateApp("{{credits}} credits", {
                  credits: formatExactCredits(wallet.available),
                })
          }
        valueTone={wallet.available?.startsWith("-") ? "danger" : "default"}
        accessibilityLabel={translateApp("Available credits: {{credits}}", {
            credits: formatCredits(wallet.available),
          })} />
      {credits.map((row) => (
          <SettingsRow key={row.key} label={row.label} value={`${row.value}${row.unit === "" ? "" : ` ${row.unit ?? "credits"}`}`}
        valueTone={row.value.startsWith("-") ? "danger" : "default"}
        accessibilityLabel={(row.unit === "" ? translateApp("{{label}}: {{amount}}", { label: row.label, amount: row.exactValue ?? row.value}) : translateApp("{{label}}: {{amount}} {{unit}}", { label: row.label, amount: row.exactValue ?? row.value, unit: row.unit ?? translateApp("credits") }))} />
        ))}
    </SettingsGroup>
    {[...groups].map(([group, items]) => (
        <SettingsGroup key={group} title={translateApp("Included · {{group}}", { group: group})} inset={false}>
      {items.map((row) => (
            <SettingsRow key={row.key} label={row.item ?? row.label} value={row.left ?? row.value} />))}
    </SettingsGroup>))}
  </>
  );
}

export function BillingServiceCard({ service }: { service: ReturnType<typeof serviceUsage>[number];
}) {
  useAppLanguage();
  return (
    <SettingsRow label={service.name} detail={service.meteringOnly ? translateApp("Metering only") : undefined}
    value={service.cost === null ? translateApp("Unavailable")
          : translateApp("{{credits}} credits", {
              credits: formatExactCredits(service.cost),
            })
      }
    accessibilityLabel={(service.cost === null ? translateApp("{{service}}: unavailable", { service: service.name }) : translateApp("{{service}}: {{credits}} credits", { service: service.name, credits: formatCredits(service.cost)}))} />
  );
}

export function BillingHistoryRow({ expanded, onPress }: { expanded: boolean; onPress: () => void;
}) {
  useAppLanguage();
  return (
    <SettingsGroup inset={false}>
    <SettingsRow label={translateApp("Usage history")}
        value={expanded ? translateApp("Hide") : undefined} accessibilityState={{ expanded }} onPress={onPress} />
  </SettingsGroup>
  );
}
