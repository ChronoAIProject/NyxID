import { useUiTranslation, translateUi } from "@/i18n/use-ui-translation";
import type {
  CertifiedContract,
  UsageContract,
} from "@/schemas/model-contracts";
import { titleCase } from "@/schemas/platform-models";
import { Checkbox } from "@/components/ui/checkbox";
import { NativeSelect } from "@/components/ui/native-select";
import { Field } from "./shared";

export function ContractPicker({
  definitions,
  value,
  onChange,
}: {
  readonly definitions: CertifiedContract[];
  readonly value: UsageContract;
  readonly onChange: (value: UsageContract) => void;
}) {
  useUiTranslation();
  const definition = definitions.find((d) => d.version === value.version)!;
  return (
    <div className="space-y-3">
      <Field title={translateUi("Certified contract")}>
        <NativeSelect
          value={value.version}
          onChange={(e) => {
            const selected = definitions.find(
              (d) => d.version === e.target.value,
            );
            if (selected) onChange(selected.default_contract);
          }}
        >
          {definitions.map((d) => (
            <option key={d.version} value={d.version}>
              {d.version}
            </option>
          ))}
        </NativeSelect>
      </Field>
      <p className="type-secondary text-muted-foreground">
        {translateUi("Transports: {{transports}}. Token and cache-read accounting are required. Defaults include all supported cache-write TTLs; verify and price every selected class.", { transports: definition.transports.join(", ") })}
      </p>
      <details>
        <summary className="type-secondary cursor-pointer">
          {translateUi("Fixture digest")}
        </summary>
        <p className="type-code break-all">{definition.fixture_sha256}</p>
      </details>
      {Object.entries(definition.flags)
        .filter(([, rule]) => rule === "optional")
        .map(([key]) => {
          const flag = key as keyof CertifiedContract["flags"];
          return (
            <Field key={flag} title={translateUi(titleCase(flag))}>
              <Checkbox
                checked={value[flag]}
                onCheckedChange={(checked) =>
                  onChange({ ...value, [flag]: checked === true })
                }
              />
            </Field>
          );
        })}
      {definition.cache_write_ttls.map((ttl) => (
        <Field key={ttl} title={translateUi("Cache writes: {{ttl}}", { ttl: translateUi(titleCase(ttl)) })}>
          <Checkbox
            checked={value.cache_write_ttls.includes(ttl)}
            onCheckedChange={(checked) =>
              onChange({
                ...value,
                cache_write_ttls:
                  checked === true
                    ? [...value.cache_write_ttls, ttl]
                    : value.cache_write_ttls.filter((t) => t !== ttl),
              })
            }
          />
        </Field>
      ))}
      {definition.input_modalities.map((modality) => (
        <Field key={modality} title={translateUi("Separate {{modality}} input accounting", { modality: translateUi(titleCase(modality)) })}>
          <Checkbox
            checked={value.input_modalities.includes(modality)}
            onCheckedChange={(checked) =>
              onChange({
                ...value,
                input_modalities:
                  checked === true
                    ? [...value.input_modalities, modality]
                    : value.input_modalities.filter((m) => m !== modality),
              })
            }
          />
        </Field>
      ))}
      <p className="type-secondary text-muted-foreground">
        {translateUi("Accounting options do not enable model features. Prices and request bounds must match your verified model limits.")}
      </p>
    </div>
  );
}
