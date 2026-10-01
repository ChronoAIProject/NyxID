import { MachineIsolationBadge, MachineIsolationDetails } from "@/components/shared/machine-isolation";
import { Link } from "@tanstack/react-router";
import { DetailSection } from "@/components/shared/detail-section";
import type { NodeInfo } from "@/types/nodes";

export function MachineSummary({ node }: { readonly node: NodeInfo }) {
  const machine = node.machine;
  if (!machine) return null;
  return (
    <DetailSection title="Machine access">
      <div className="space-y-3 text-[12px]">
        <p>
          {[
            machine.shell && "Commands",
            machine.files && "Files",
            machine.computer && "Computer",
          ]
            .filter(Boolean)
            .join(" · ")}
        </p>
        <MachineIsolationBadge machine={machine} />
        <MachineIsolationDetails machine={machine} />
        <p>Owner confirmation: {node.machine_confirm ?? "none"}</p>
        <Link to="/assistant/machines" className="text-primary underline">
          Manage in Assistant → Machines
        </Link>
      </div>
    </DetailSection>
  );
}
