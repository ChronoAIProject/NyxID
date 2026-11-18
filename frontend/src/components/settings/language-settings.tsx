import { LANGUAGES } from "@/i18n/language";
import { useAppTranslation } from "@/i18n/use-app-translation";
import { Label } from "@/components/ui/label";
import { ChevronDown } from "lucide-react";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
export function LanguageSettings() {
  const { t, i18n } = useAppTranslation();
  return (
    <Card>
      <CardHeader>
        <CardTitle>
          <Label htmlFor="interface-language">{t("Language")}</Label>
        </CardTitle>
        <CardDescription>
          {t(
            "Choose the language for Studio and NyxBot. Saved in this browser.",
          )}
        </CardDescription>
      </CardHeader>
      <CardContent>
        <div className="relative w-full sm:w-64">
          <select
            id="interface-language"
            value={i18n.language}
            onChange={(event) => void i18n.changeLanguage(event.target.value)}
            className="min-h-control h-9 w-full appearance-none rounded-md border border-input bg-background py-1.5 pl-3 pr-9 text-12 text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            {LANGUAGES.map((language) => (
              <option
                key={language.code}
                value={language.code}
                lang={language.code}
              >
                {language.label}
              </option>
            ))}
          </select>
          <ChevronDown
            aria-hidden="true"
            className="pointer-events-none absolute right-3 top-1/2 size-3.5 -translate-y-1/2 text-text-tertiary"
          />
        </div>
      </CardContent>
    </Card>
  );
}
