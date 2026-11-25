import { Link } from "@tanstack/react-router";
import { NyxidIcon } from "@/components/brand/nyxid-icon";
import { Button } from "@/components/ui/button";
import { useDocumentTitle } from "@/hooks/use-document-title";
import { useUiTranslation } from "@/i18n/use-ui-translation";

export function ActivityPage() {
  const { t } = useUiTranslation();
  const title = t("Activity");
  useDocumentTitle(title);

  return (
    <main className="flex min-h-dvh items-center justify-center bg-background px-6">
      <section className="flex w-full max-w-sm flex-col items-center gap-4 rounded-lg border border-border bg-card p-6 text-center">
        <NyxidIcon className="h-8 w-8" />
        <h1 className="type-section-heading">{title}</h1>
        <Button asChild variant="primary" size="lg" className="w-full">
          <a href="nyxid://activity">{t("Open NyxID app")}</a>
        </Button>
        <Link to="/assistant/approvals" className="type-secondary text-muted-foreground underline">
          {t("Review approvals on the web")}
        </Link>
      </section>
    </main>
  );
}
