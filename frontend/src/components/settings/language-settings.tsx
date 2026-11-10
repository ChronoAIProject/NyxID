import { LANGUAGES } from "@/i18n/language";
import { useAppTranslation } from "@/i18n/use-app-translation";
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
          <label htmlFor="interface-language">{t("Language")}</label>
        </CardTitle>
        <CardDescription>
          {t(
            "Choose the language for Studio and NyxBot. Saved in this browser.",
          )}
        </CardDescription>
      </CardHeader>
      <CardContent>
        <select
          id="interface-language"
          value={i18n.language}
          onChange={(event) => void i18n.changeLanguage(event.target.value)}
          className="min-h-control h-9 w-full rounded-md border border-input bg-background px-3 text-12 text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring sm:w-64"
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
      </CardContent>
    </Card>
  );
}
