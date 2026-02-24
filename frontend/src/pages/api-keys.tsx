import { ApiKeyTable } from "@/components/dashboard/api-key-table";
import { ApiKeyCreateDialog } from "@/components/dashboard/api-key-create-dialog";

export function ApiKeysPage() {
  return (
    <div className="space-y-8">
      <div className="flex items-center justify-between">
        <div>
          <h2 className="font-display text-5xl font-normal tracking-tight">API Keys</h2>
          <p className="text-muted-foreground">
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
