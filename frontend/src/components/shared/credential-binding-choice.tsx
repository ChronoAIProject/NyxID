import { lanePriceLabel, type LanePricingView } from "@/schemas/platform-keys";
import { cn } from "@/lib/utils";

export function CredentialBindingChoice({
  value,
  onChange,
  platformPrice,
  byokPrice,
  legacyBillable = false,
  resaleBillable = false,
  disabled = false,
}: {
  readonly value: boolean;
  readonly onChange: (platform: boolean) => void;
  readonly platformPrice?: LanePricingView | null;
  readonly byokPrice?: LanePricingView | null;
  readonly disabled?: boolean;
  readonly legacyBillable?: boolean;
  readonly resaleBillable?: boolean;
}) {
  return (
    <fieldset className="space-y-2" disabled={disabled}>
      <legend className="mb-2 text-xs font-medium">Choose a key</legend>
      {[
        { platform: true, title: "Use NyxID's key", price: platformPrice },
        { platform: false, title: "Use your own key", price: byokPrice },
      ].map((option) => (
        <label
          key={String(option.platform)}
          className={cn(
            "flex cursor-pointer items-start gap-3 rounded-lg border p-3 text-xs",
            value === option.platform
              ? "border-primary/50"
              : "border-border/50",
          )}
        >
          <input
            type="radio"
            name="credential-binding"
            checked={value === option.platform}
            onChange={() => onChange(option.platform)}
            className="mt-0.5 accent-primary"
          />
          <span>
            <span className="block font-medium">{option.title}</span>
            <span className="text-muted-foreground">
              {!platformPrice && !byokPrice && legacyBillable
                ? "Current service/plan pricing applies"
                : lanePriceLabel(option.price)}
            </span>
          </span>
        </label>
      ))}
      {resaleBillable && (
        <p className="text-[11px] text-muted-foreground">
          Platform-key use may also incur the separate resale fee.
        </p>
      )}
    </fieldset>
  );
}
