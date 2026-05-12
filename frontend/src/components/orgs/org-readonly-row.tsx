interface OrgReadOnlyRowProps {
  readonly orgName: string;
}

export function OrgReadOnlyRow({ orgName }: OrgReadOnlyRowProps) {
  return (
    <div className="rounded-lg border border-border bg-muted px-3 py-2">
      <p className="text-xs uppercase tracking-wide text-text-tertiary">
        Organization
      </p>
      <p className="text-[12px] font-medium text-foreground">{orgName}</p>
    </div>
  );
}
