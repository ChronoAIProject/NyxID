import { ApiKeyTable } from "@/components/dashboard/api-key-table";
import { ApiKeyCreateDialog } from "@/components/dashboard/api-key-create-dialog";

export function ApiKeysPage() {
  return (
    <div className="space-y-8">
      <div className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h2 className="font-display text-3xl font-normal tracking-tight md:text-5xl">API Keys</h2>
          <p className="text-sm text-muted-foreground">
            Manage your API keys for programmatic access.
          </p>
        </div>
        <ApiKeyCreateDialog />
      </div>

      <div className="rounded-xl border border-border">
        <ApiKeyTable />
      </div>
    </div>
  );
}
