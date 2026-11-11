import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { KeyRound } from "lucide-react";
import { NyxidIcon } from "@/components/brand/nyxid-icon";
import { ChoiceCard, ChoiceCardGroup } from "@/components/shared/choice-card";
import { lanePriceLabel, type LanePricingView } from "@/schemas/platform-keys";

export function CredentialBindingChoice({
  value,
  onChange,
  platformPrice,
  byokPrice,
  legacyBillable = false,
  resaleBillable = false,
  disabled = false,
  showPricing = true,
}: {
  readonly value: boolean;
  readonly onChange: (platform: boolean) => void;
  readonly platformPrice?: LanePricingView | null;
  readonly byokPrice?: LanePricingView | null;
  readonly disabled?: boolean;
  readonly legacyBillable?: boolean;
  readonly resaleBillable?: boolean;
  /** The CLI wizard keeps cost out of this step and shows what each key means. */
  readonly showPricing?: boolean;
}) {
  useUiTranslation();
  const priceLabel = (price: LanePricingView | null | undefined) =>
    !platformPrice && !byokPrice && legacyBillable
      ? "Current service/plan pricing applies"
      : lanePriceLabel(price);
  return (
    <ChoiceCardGroup
      label={translateUi("Choose a key")}
      ariaLabel={translateUi("Credential")}
      footer={
        showPricing && resaleBillable ? (
          <p className="text-13 text-muted-foreground">{translateUi("Platform-key use may also incur the separate resale fee.")}</p>
        ) : null
      }
    >
      <ChoiceCard
        checked={value}
        disabled={disabled}
        onSelect={() => onChange(true)}
        icon={<NyxidIcon className="h-5 w-5" alt="" />}
        title={translateUi("Use NyxID's key")}
        description={
          showPricing
            ? priceLabel(platformPrice)
            : translateUi("NyxID supplies the key. Nothing to paste.")
        }
      />
      <ChoiceCard
        checked={!value}
        disabled={disabled}
        onSelect={() => onChange(false)}
        icon={<KeyRound className="h-5 w-5" />}
        title={translateUi("Use your own key")}
        description={
          showPricing
            ? priceLabel(byokPrice)
            : translateUi("Use a key from your own account.")
        }
      />
    </ChoiceCardGroup>
  );
}
