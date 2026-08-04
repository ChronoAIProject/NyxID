import { ConnectionGrid } from "@/components/dashboard/connection-grid";

export function ConnectionsPage() {
  return (
    <div className="space-y-8">
      <div>
        <h2 className="text-[28px] font-bold leading-none tracking-tight" style={{ letterSpacing: "-0.03em" }}>
          Connections
        </h2>
        <p className="text-[12px] text-muted-foreground">
          Manage your connections to downstream services.
        </p>
      </div>

      <ConnectionGrid />
    </div>
  );
}
