interface OrgReadOnlyRowProps {
  readonly orgName: string;
}

export function OrgReadOnlyRow({ orgName }: OrgReadOnlyRowProps) {
  return (
    <div className="rounded-lg border border-border bg-muted px-3 py-2">
      <p className="text-13 uppercase tracking-wide text-text-tertiary">
        Organization
      </p>
      <p className="type-row-title text-foreground">{orgName}</p>
    </div>
  );
}
