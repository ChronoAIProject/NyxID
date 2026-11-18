import { useUiTranslation } from "@/i18n/use-ui-translation";
import { LegalDocumentPage } from "./_legal-document-page";

export function TermsPage() {
  const { t } = useUiTranslation();
  return (
    <LegalDocumentPage
      title={t("Terms of Service")}
      mdPath="/legal/terms.md"
      docKey="terms"
    />
  );
}
