import { Languages } from "lucide-react-native";
import type { SheetScreenProps } from "../../app/navigationTypes";
import {
  SettingsGroup,
  SettingsMenuRow,
  SettingsPage,
  SettingsRow,
} from "../../components/settings/SettingsList";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { LANGUAGES, type Language } from "../../i18n/language";

export function LanguageSettingsScreen(
  _props: SheetScreenProps<"LanguageSettings">,
) {
  const { language, choose, persistenceError, retryPersistence, t } =
    useAppLanguage();
  return (
    <SettingsPage>
      <SettingsGroup
        title={t("App language")}
        footer={t(
          "Saved on this device. Dictation language is set separately in Voice settings. Messages stay in their original language.",
        )}
      >
        <SettingsMenuRow
          icon={Languages}
          label={t("Language")}
          value={language}
          options={LANGUAGES.map(({ code, label }) => ({ value: code, label }))}
          onChange={(value) => {
            if (LANGUAGES.some(({ code }) => code === value))
              choose(value as Language);
          }}
        />
      </SettingsGroup>
      {persistenceError ? (
        <SettingsGroup
          footer={t(
            "This language could not be saved. It is active for this session; retry before closing the app.",
          )}
        >
          <SettingsRow
            label={t("Retry saving language")}
            onPress={retryPersistence}
          />
        </SettingsGroup>
      ) : null}
    </SettingsPage>
  );
}
