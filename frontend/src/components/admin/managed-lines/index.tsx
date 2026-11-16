import { Link } from "@tanstack/react-router";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";
import { useAuthStore } from "@/stores/auth-store";
import { AccountsSection } from "./accounts-section";
import { EligibilitySection } from "./eligibility-section";
import { InventorySection } from "./inventory-section";

/** Platform-admin managed phone lines: eligibility, custody inventory and accounts. */
export function ManagedLinesAdmin() {
  useUiTranslation();
  const user = useAuthStore((s) => s.user);
  if (!user?.is_admin) return null;
  return (
    <section
      key={user.id}
      aria-label={translateUi("NyxID-managed phone lines")}
      className="space-y-6 rounded-xl border border-border bg-card p-5 text-14"
    >
      <div>
        <h2 className="type-subheading">{translateUi("NyxID-managed phone lines")}</h2>
        <p className="mt-2 text-muted-foreground">{translateUi("Resell an outcome, not vendor API access: approved merchants claim a NyxID-owned number answered by their specialist and pay in credits. Managed credentials never reach generic proxy, MCP, LLM or key provisioning.")}</p>
        <Link to="/admin/managed-lines/report" className="mt-2 inline-block underline">
          {translateUi("Open the managed lines margin report")}
        </Link>
      </div>
      <EligibilitySection />
      <InventorySection />
      <AccountsSection />
    </section>
  );
}
