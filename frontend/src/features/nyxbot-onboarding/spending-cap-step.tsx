import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import {
  OnboardingShell,
  OnboardingNotice,
  StepHelp,
  BackButton,
} from "./onboarding-shell";

export function SpendingCapStep({ onBack }: { readonly onBack: () => void }) {
  const { t } = useTranslation();
  const [cap, setCap] = useState("120");
  return (
    <OnboardingShell
      title={t("capTitle")}
      subtitle={`${t("step", { step: 3 })} · WhatsApp Business`}
      step={3}
      actions={
        <>
          <Button className="nb-primary" disabled>
            {t("capSubmit")}
          </Button>
          <BackButton onClick={onBack} />
        </>
      }
    >
      <p className="nb-intro">
        {t("capDescription")} <StepHelp>{t("capHelp")}</StepHelp>
      </p>
      <fieldset className="nb-cap-options">
        <legend className="sr-only">{t("capTitle")}</legend>
        {[
          { value: "120", title: "cap120" },
          { value: "250", title: "cap250" },
          { value: "none", title: "capNone" },
        ].map((option) => (
          <label
            key={option.value}
            className="nb-cap-option"
            data-selected={cap === option.value}
          >
            <input
              type="radio"
              name="spending-cap"
              value={option.value}
              checked={cap === option.value}
              onChange={() => setCap(option.value)}
            />
            <span>{t(option.title)}</span>
            {option.value === "120" && <small>{t("recommended")}</small>}
          </label>
        ))}
      </fieldset>
      <OnboardingNotice>{t("capUnavailable")}</OnboardingNotice>
    </OnboardingShell>
  );
}
