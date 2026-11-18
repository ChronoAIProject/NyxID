import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp } from "../../i18n/appTranslation";
import type { SheetScreenProps } from "../../app/navigationTypes";
import { LegalDocumentScreen } from "./LegalDocumentScreen";

type Props = SheetScreenProps<"TermsOfService">;

export function TermsOfServiceScreen({ navigation }: Props) {
  useAppLanguage();
  return (
    <LegalDocumentScreen
      title={translateApp("Terms of Service")}
      docKey="terms"
      telemetryBackTarget="terms_back"
      onBack={() => navigation.goBack()}
      onOpenDocument={(docKey) => {
        if (docKey !== "privacy") return;
        if (
          navigation
            .getState()
            .routes.some((route) => route.name === "PrivacyPolicy")
        )
          navigation.popTo("PrivacyPolicy");
        else navigation.push("PrivacyPolicy");
      }}
    />
  );
}
