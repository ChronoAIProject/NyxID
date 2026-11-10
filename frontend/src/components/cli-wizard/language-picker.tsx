import type { KeyboardEvent } from "react";
import { Languages } from "lucide-react";
import { LANGUAGES } from "@/i18n/language";
import { useWizardTranslation } from "@/i18n/wizard";
import { cn } from "@/lib/utils";

export function WizardLanguagePicker() {
  const { t, i18n } = useWizardTranslation();
  const selected =
    LANGUAGES.find(({ code }) => code === i18n.resolvedLanguage) ??
    LANGUAGES[0];
  function handleKeyDown(event: KeyboardEvent<HTMLButtonElement>, index: number) {
    if (event.altKey || event.ctrlKey || event.metaKey) return;
    let next: number;
    switch (event.key) {
      case "ArrowRight":
      case "ArrowDown":
        next = (index + 1) % LANGUAGES.length;
        break;
      case "ArrowLeft":
      case "ArrowUp":
        next = (index + LANGUAGES.length - 1) % LANGUAGES.length;
        break;
      case "Home":
        next = 0;
        break;
      case "End":
        next = LANGUAGES.length - 1;
        break;
      default:
        return;
    }
    event.preventDefault();
    event.currentTarget.parentElement
      ?.querySelectorAll<HTMLButtonElement>('[role="radio"]')[next]
      ?.focus();
    void i18n.changeLanguage(LANGUAGES[next]!.code);
  }

  return (
    <div
      role="radiogroup"
      aria-label={t("Language")}
      className="inline-flex shrink-0 items-center gap-0.5 rounded-xl border border-border/50 bg-muted/40 p-1"
    >
      <span
        aria-hidden="true"
        className="flex w-6 items-center justify-center text-foreground"
      >
        <Languages className="size-4" />
      </span>
      {LANGUAGES.map(({ code, label }, index) => {
        const prefix = code === "en" ? "EN" : code === "zh-CN" ? "CN" : "ZH";
        return (
          <button
            key={code}
            type="button"
            role="radio"
            aria-checked={selected.code === code}
            aria-label={label}
            title={label}
            lang={code}
            tabIndex={selected.code === code ? 0 : -1}
            onClick={() => void i18n.changeLanguage(code)}
            onKeyDown={(event) => handleKeyDown(event, index)}
            className={cn(
              "h-8 min-w-9 cursor-pointer rounded-lg px-2 text-12 font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background",
              selected.code === code
                ? "bg-card text-foreground shadow-sm ring-1 ring-border/60"
                : "text-muted-foreground hover:bg-card/60 hover:text-foreground",
            )}
          >
            {prefix}
          </button>
        );
      })}
    </div>
  );
}
