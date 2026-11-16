import { useEffect, useState, type ComponentProps } from "react";
import { Keyboard } from "react-native";
import type { LucideIcon } from "lucide-react-native";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import {
  WorkspaceSelectionSheet,
  type WorkspaceSelectionOption,
} from "../workspace/WorkspaceSelectionField";
import { SettingsRow } from "./SettingsList";

type SheetProps = Omit<
  ComponentProps<typeof WorkspaceSelectionSheet>,
  "onClose"
>;

const signature = (value: readonly string[]) =>
  JSON.stringify([...new Set(value)].sort());

/** Summary of the selected labels, as the row's value. */
export function selectionSummary(
  options: readonly WorkspaceSelectionOption[],
  value: readonly string[],
  placeholder = "None",
) {
  const labels = new Map(options.map((option) => [option.value, option.label]));
  const selected = [...new Set(value)];
  if (!selected.length) return placeholder;
  if (selected.length > 2) return `${selected.length} selected`;
  return selected.map((id) => labels.get(id) ?? id).join(", ");
}

/**
 * A label/value row that opens the searchable selection sheet for a longer
 * choice list. Like the form field it replaces, Apply edits only the enclosing
 * draft; the sheet closes when its scope, value or availability changes.
 */
export function SettingsSelectionRow({
  detail,
  icon,
  ...props
}: SheetProps & { detail?: string; icon?: LucideIcon }) {
  const { t } = useAppLanguage();
  const [open, setOpen] = useState(false);
  const valueKey = signature(props.value);
  useEffect(() => {
    setOpen(false);
  }, [props.scopeKey, valueKey, props.disabled, props.readOnly]);
  const blocked = !!props.disabled && !props.readOnly;
  const value =
    props.summary ??
    selectionSummary(props.options, props.value, props.placeholder);
  return (
    <>
      <SettingsRow
        icon={icon}
        label={props.label}
        value={value}
        detail={detail}
        disabled={blocked}
        accessibilityState={{ expanded: open }}
        accessibilityHint={
          props.readOnly
            ? t("Shows {{label}}", { label: props.label })
            : t("Changes {{label}}", { label: props.label })
        }
        onPress={() => {
          if (blocked) return;
          Keyboard.dismiss();
          setOpen(true);
        }}
      />
      {open && (
        <WorkspaceSelectionSheet {...props} onClose={() => setOpen(false)} />
      )}
    </>
  );
}
