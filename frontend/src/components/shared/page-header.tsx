import { Breadcrumb, type BreadcrumbItem } from "./breadcrumb";

interface PageHeaderProps {
  readonly breadcrumbs?: readonly BreadcrumbItem[];
  readonly title: string;
  readonly description?: string;
  readonly actions?: React.ReactNode;
}

/* ── VoidPortal Page Header ── */
export function PageHeader({
  breadcrumbs,
  title,
  description,
  actions,
}: PageHeaderProps) {
  return (
    <div className="flex flex-col gap-2">
      {breadcrumbs && breadcrumbs.length > 0 && (
        <Breadcrumb items={breadcrumbs} />
      )}
      <div className="flex items-center justify-between">
        <div className="flex flex-col gap-2">
          <h2 className="font-display text-5xl font-normal tracking-tight">
            {title}
          </h2>
          {description && (
            <p className="text-sm text-muted-foreground">{description}</p>
          )}
        </div>
        {actions && <div className="flex items-center gap-2">{actions}</div>}
      </div>
    </div>
  );
}
