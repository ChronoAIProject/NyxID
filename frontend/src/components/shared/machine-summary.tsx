import { useUiTranslation } from "@/i18n/use-ui-translation";
import { MachineIsolationBadge, MachineIsolationDetails } from "@/components/shared/machine-isolation";
import { Link } from "@tanstack/react-router";
import { DetailSection } from "@/components/shared/detail-section";
import type { NodeInfo } from "@/types/nodes";

export function MachineSummary({ node }: { readonly node: NodeInfo }) {
  const { t } = useUiTranslation();
  const machine = node.machine;
  if (!machine) return null;
  return (
    <DetailSection title={t("Machine access")}>
      <div className="space-y-3 text-12">
        <p>
          {[
            machine.shell && "Commands",
            machine.files && "Files",
            (machine.browser ?? machine.computer) && "Browser",
                machine.computer && "Computer",
          ]
            .filter(Boolean)
            .join(" · ")}
        </p>
        <MachineIsolationBadge machine={machine} />
        <MachineIsolationDetails machine={machine} />
        <p>{t("Owner confirmation: ")}{node.machine_confirm ?? t("none")}</p>
        <Link to="/assistant/machines" className="text-primary-text underline">{t("Manage in Assistant → Machines")}</Link>
      </div>
    </DetailSection>
  );
}
