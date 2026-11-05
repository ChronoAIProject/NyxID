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
  const priceLabel = (price: LanePricingView | null | undefined) =>
    !platformPrice && !byokPrice && legacyBillable
      ? "Current service/plan pricing applies"
      : lanePriceLabel(price);
  return (
    <ChoiceCardGroup
      label="Choose a key"
      ariaLabel="Credential"
      footer={
        showPricing && resaleBillable ? (
          <p className="text-13 text-muted-foreground">
            Platform-key use may also incur the separate resale fee.
          </p>
        ) : null
      }
    >
      <ChoiceCard
        checked={value}
        disabled={disabled}
        onSelect={() => onChange(true)}
        icon={<NyxidIcon className="h-5 w-5" alt="" />}
        title="Use NyxID's key"
        description={
          showPricing
            ? priceLabel(platformPrice)
            : "NyxID supplies the key. Nothing to paste."
        }
      />
      <ChoiceCard
        checked={!value}
        disabled={disabled}
        onSelect={() => onChange(false)}
        icon={<KeyRound className="h-5 w-5" />}
        title="Use your own key"
        description={
          showPricing
            ? priceLabel(byokPrice)
            : "Use a key from your own account."
        }
      />
    </ChoiceCardGroup>
  );
}
