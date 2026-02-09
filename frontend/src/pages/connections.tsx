import { ConnectionGrid } from "@/components/dashboard/connection-grid"

export function ConnectionsPage() {
  return (
    <div className="space-y-6">
      <div>
        <h2 className="text-3xl font-bold tracking-tight">Connections</h2>
        <p className="text-muted-foreground">
          Manage your connections to downstream services.
        </p>
      </div>

      <ConnectionGrid />
    </div>
  )
}
