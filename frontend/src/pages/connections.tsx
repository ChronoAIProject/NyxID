import { ConnectionGrid } from "@/components/dashboard/connection-grid";

export function ConnectionsPage() {
  return (
    <div className="space-y-8">
      <div>
        <h2 className="font-display text-3xl md:text-5xl font-normal tracking-tight">Connections</h2>
        <p className="text-sm text-muted-foreground">
          Manage your connections to downstream services.
        </p>
      </div>

      <ConnectionGrid />
    </div>
  );
}
