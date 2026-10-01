import { Link } from "@tanstack/react-router";
import { Badge } from "@/components/ui/badge";
import { DetailSection } from "@/components/shared/detail-section";
import { SINGLE_USER_SHELL_WARNING } from "@/schemas/machines";
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
        {machine.shell && !machine.browser_isolated ? (
          <>
            <Badge variant="warning">Not isolated</Badge>
            <p className="text-muted-foreground">{SINGLE_USER_SHELL_WARNING}</p>
          </>
        ) : null}
        <p>Owner confirmation: {node.machine_confirm ?? "none"}</p>
        <Link to="/assistant/machines" className="text-primary underline">
          Manage in Assistant → Machines
        </Link>
      </div>
    </DetailSection>
  );
}
