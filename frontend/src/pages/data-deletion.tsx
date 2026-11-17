import { useUiTranslation } from "@/i18n/use-ui-translation";
import { LegalDocumentPage } from "./_legal-document-page";

export function DataDeletionPage() {
  const { t } = useUiTranslation();
  return (
    <LegalDocumentPage
      title={t("Data Deletion Instructions")}
      mdPath="/legal/data-deletion.md"
      docKey="data-deletion"
    />
  );
}
