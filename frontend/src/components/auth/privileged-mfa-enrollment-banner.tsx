import { useState } from "react";
import { ShieldCheck } from "lucide-react";
import { useUiTranslation } from "@/i18n/use-ui-translation";
import { MfaSetupDialog } from "@/components/auth/mfa-setup-dialog";
import { Button } from "@/components/ui/button";
import { useAuthStore } from "@/stores/auth-store";
import {
  canCreateFeatureFlags,
  canManageCatalog,
  canManageCreditGrants,
  canManageTools,
  canReadAdminUsage,
  canReadFeatureFlags,
  type User,
} from "@/types/api";

export function PrivilegedMfaEnrollmentBanner() {
  const user = useAuthStore((state) => state.user);
  return user && <AccountEnrollment key={user.id} user={user} />;
}

function AccountEnrollment({ user }: { readonly user: User }) {
  const [open, setOpen] = useState(false);
  const { t } = useUiTranslation();
  const needsEnrollment =
    user.mfa_enabled === false &&
    (canReadAdminUsage(user) ||
      canManageCreditGrants(user) ||
      canManageCatalog(user) ||
      canManageTools(user) ||
      canReadFeatureFlags(user) ||
      canCreateFeatureFlags(user));

  return (
    <>
      {needsEnrollment && (
        <div className="flex flex-wrap items-center gap-3 rounded-lg border border-warning/30 bg-warning/5 px-4 py-3">
          <ShieldCheck className="h-5 w-5 shrink-0 text-warning" aria-hidden="true" />
          <div className="min-w-0 flex-1 basis-48" role="status">
            <p className="type-row-title text-foreground">{t("MFA required for admin access")}</p>
            <p className="type-secondary text-muted-foreground">
              {t("Set up an authenticator app to keep your admin access. Enrolling signs you out on your other devices.")}
            </p>
          </div>
          <Button variant="primary" size="sm" onClick={() => setOpen(true)}>
            {t("Set up MFA")}
          </Button>
        </div>
      )}
      {open && <MfaSetupDialog open onOpenChange={setOpen} />}
    </>
  );
}
