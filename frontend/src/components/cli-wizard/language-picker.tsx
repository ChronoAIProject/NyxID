import { Trigger } from "@radix-ui/react-select";
import { Globe } from "lucide-react";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectValue,
} from "@/components/ui/select";
import { LANGUAGES } from "@/i18n/language";
import { useWizardTranslation } from "@/i18n/wizard";

export function WizardLanguagePicker() {
  const { t, i18n } = useWizardTranslation();
  const selected =
    LANGUAGES.find(({ code }) => code === i18n.resolvedLanguage) ??
    LANGUAGES[0];
  const prefix =
    selected.code === "en" ? "EN" : selected.code === "zh-CN" ? "CN" : "ZH";
  return (
    <Select
      value={selected.code}
      onValueChange={(language) => void i18n.changeLanguage(language)}
    >
      <Trigger
        aria-label={t("Language")}
        title={selected.label}
        className="inline-flex h-10 shrink-0 cursor-pointer items-center justify-center gap-2 rounded-full border border-input bg-card px-4 text-12 font-medium text-foreground shadow-md transition-colors hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background"
      >
        <span aria-hidden="true">{prefix}</span>
        <Globe className="size-4" aria-hidden="true" />
        <span className="sr-only">
          <SelectValue />
        </span>
      </Trigger>
      <SelectContent align="end" className="min-w-40">
        {LANGUAGES.map(({ code, label }) => (
          <SelectItem key={code} value={code} lang={code}>
            {label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}
