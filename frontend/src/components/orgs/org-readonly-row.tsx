import { useUiTranslation } from "@/i18n/use-ui-translation";
interface OrgReadOnlyRowProps {
  readonly orgName: string;
}

export function OrgReadOnlyRow({ orgName }: OrgReadOnlyRowProps) {
  const { t } = useUiTranslation();
  return (
    <div className="rounded-lg border border-border bg-muted px-3 py-2">
      <p className="text-xs uppercase tracking-wide text-text-tertiary">{t("Organization")}</p>
      <p className="text-12 font-medium text-foreground">{orgName}</p>
    </div>
  );
}
