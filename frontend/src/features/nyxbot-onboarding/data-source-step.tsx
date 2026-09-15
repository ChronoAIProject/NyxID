import type { ReactNode } from "react";
import {
  ArrowLeft,
  CalendarDays,
  FileText,
  Info,
  ShieldCheck,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { BrandIcon } from "./brand-icon";
import { OnboardingShell } from "./onboarding-shell";

export function DataSourceStep({
  accountName,
  ready,
  disabled,
  connecting,
  onConnect,
  onBack,
  children,
}: {
  readonly accountName: string;
  readonly ready: boolean;
  readonly disabled: boolean;
  readonly connecting: boolean;
  readonly onConnect: () => void;
  readonly onBack: () => void;
  readonly children: ReactNode;
}) {
  const { t } = useTranslation();
  return (
    <OnboardingShell
      variant="setup"
      title={t("sourceTitle")}
      subtitle={t("sourceIntro")}
      step={2}
      actions={
        <>
          <Button
            className="nb-secondary"
            onClick={onBack}
            disabled={connecting}
          >
            <ArrowLeft size={20} aria-hidden="true" />
            {t("back")}
          </Button>
          <Button
            className="nb-primary"
            isLoading={connecting}
            disabled={disabled}
            onClick={onConnect}
          >
            {!ready && !connecting && <BrandIcon brand="google" />}
            {t(ready ? "continue" : "connectGoogle")}
          </Button>
        </>
      }
    >
      <div className="nb-source-account">
        <span>
          <ShieldCheck size={18} aria-hidden="true" />
          {t("sourceSignedIn")}
        </span>
        <span className="nb-source-account-name">{accountName}</span>
      </div>
      <div className="nb-source-options">
        <div className="nb-source-option" data-selected="true">
          <BrandIcon brand="google" />
          <strong>{t("workspace")}</strong>
          <span>{t("workspaceServices")}</span>
        </div>
        <div className="nb-source-option" aria-disabled="true">
          <BrandIcon brand="notion" />
          <strong>Notion</strong>
          <span>{t("sourceComingSoon")}</span>
        </div>
      </div>
      <ul className="nb-source-services">
        <li>
          <FileText size={20} aria-hidden="true" />
          <div>
            <strong>Google Drive</strong>
            <p>{t("driveDescription")}</p>
          </div>
        </li>
        <li>
          <CalendarDays size={20} aria-hidden="true" />
          <div>
            <strong>Google Calendar</strong>
            <p>{t("calendarDescription")}</p>
          </div>
        </li>
      </ul>
      <div className="nb-source-permissions">
        <Info size={16} aria-hidden="true" />
        <p>{t("sourcePermissions")}</p>
      </div>
      {children}
    </OnboardingShell>
  );
}
