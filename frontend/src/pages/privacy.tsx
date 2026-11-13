import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { LegalDocumentPage } from "./_legal-document-page";

export function PrivacyPage() {
  useUiTranslation();
  return (
    <LegalDocumentPage
      title={translateUi("Privacy Policy")}
      mdPath="/legal/privacy.md"
      docKey="privacy"
    />
  );
}
