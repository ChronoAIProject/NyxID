import { radioKeys } from "@/lib/radio-group";
import { Check } from "lucide-react";
import { GroupedList, GroupedRow } from "./grouped-list";

export function CheckmarkList<T extends string>({
  label,
  options,
  value,
  onChange,
  disabled,
  footer,
}: {
  readonly label: string;
  readonly options: readonly { value: T; label: string; description?: string }[];
  readonly value: T;
  readonly onChange: (value: T) => void;
  readonly disabled?: boolean;
  readonly footer?: React.ReactNode;
}) {
  return (
    <GroupedList role="radiogroup" aria-label={label} footer={footer}>
      {options.map((option, index) => (
        <GroupedRow key={option.value} asChild interactive>
          <button
            type="button"
            role="radio"
            aria-checked={option.value === value}
            disabled={disabled}
            tabIndex={
              option.value === value || (!options.some((row) => row.value === value) && index === 0)
                ? 0
                : -1
            }
            onKeyDown={radioKeys}
            onClick={() => onChange(option.value)}
          >
            <span className="min-w-0 flex-1">
              <span>{option.label}</span>
              {option.description && (
                <span className="block text-13 text-text-tertiary">{option.description}</span>
              )}
            </span>
            {option.value === value && <Check aria-hidden className="size-[1.125em] shrink-0" />}
          </button>
        </GroupedRow>
      ))}
    </GroupedList>
  );
}
