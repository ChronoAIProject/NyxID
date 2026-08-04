interface PageHeaderProps {
  readonly title: string;
  readonly description?: string;
  readonly actions?: React.ReactNode;
  readonly leading?: React.ReactNode;
}

export function PageHeader({
  title,
  description,
  actions,
  leading,
}: PageHeaderProps) {
  return (
    <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between sm:gap-4">
      <div className="flex flex-col gap-1 min-w-0">
        <div className="flex items-center gap-3">
          {leading && <div className="shrink-0">{leading}</div>}
          <h2 className="text-[22px] sm:text-[28px] font-bold leading-none tracking-tight" style={{ letterSpacing: "-0.03em" }}>
            {title}
          </h2>
        </div>
        {description && (
          <p className="text-[12px] text-muted-foreground">{description}</p>
        )}
      </div>
      {actions && <div className="flex items-center gap-2 shrink-0">{actions}</div>}
    </div>
  );
}
