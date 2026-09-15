import { useEffect, useRef, type ReactNode } from "react";
import { Check, CircleHelp, LoaderCircle, Moon, Sun } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { useApplyTheme, useResolvedTheme } from "@/hooks/use-theme";
import { useThemeStore } from "@/stores/theme-store";
import { BrandIcon } from "./brand-icon";

export function StepHelp({ children }: { readonly children: ReactNode }) {
  const { t } = useTranslation();
  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          type="button"
          className="nb-help"
          aria-label={t("help")}
          title={t("help")}
        >
          <CircleHelp aria-hidden="true" size={15} />
        </button>
      </PopoverTrigger>
      <PopoverContent
        className="nb-help-content"
        side="bottom"
        collisionPadding={16}
      >
        {children}
      </PopoverContent>
    </Popover>
  );
}

export function OnboardingNotice({
  children,
  error = false,
}: {
  readonly children: ReactNode;
  readonly error?: boolean;
}) {
  return (
    <div
      className={`nb-notice${error ? " nb-notice-error" : ""}`}
      role={error ? "alert" : "status"}
    >
      {children}
    </div>
  );
}

export function OnboardingShell({
  title,
  subtitle,
  step,
  children,
  actions,
  variant = "compact",
  complete = false,
}: {
  readonly title: string;
  readonly subtitle?: string;
  readonly step: 1 | 2 | 3 | null;
  readonly children: ReactNode;
  readonly actions: ReactNode;
  readonly variant?: "compact" | "setup";
  readonly complete?: boolean;
}) {
  const { t, i18n } = useTranslation();
  useApplyTheme();
  const theme = useResolvedTheme();
  const toggleTheme = useThemeStore((s) => s.toggle);
  const heading = useRef<HTMLHeadingElement>(null);
  useEffect(() => {
    heading.current?.focus({ preventScroll: true });
    window.scrollTo(0, 0);
  }, [title]);

  if (variant === "setup") {
    const steps = [t("accountStep"), t("dataSourceStep"), t("channelStep")];
    return (
      <div className="nyxbot-onboarding nb-setup" lang={i18n.resolvedLanguage}>
        <header className="nb-setup-topbar">
          <div className="nb-setup-topbar-inner">
            <div className="nb-setup-brand">
              <span className="nb-setup-logo" aria-hidden="true">
                NB
              </span>
              <strong>Nyxbot</strong>
              <span>{t("setup")}</span>
            </div>
            <button
              type="button"
              className="nb-setup-return"
              disabled
              aria-label={t("backToNyxbot")}
              title={t("nyxbotReturnUnavailable")}
            >
              <BrandIcon brand="telegram" />
              <span>{t("backToNyxbot")}</span>
            </button>
          </div>
        </header>
        <div className="nb-setup-shell">
          {step !== null && (
            <nav className="nb-setup-steps" aria-label={t("title")}>
              <ol>
                {steps.map((label, index) => (
                  <li
                    key={label}
                    data-state={
                      complete || index + 1 < step
                        ? "complete"
                        : index + 1 === step
                          ? "current"
                          : "pending"
                    }
                    aria-current={
                      !complete && index + 1 === step ? "step" : undefined
                    }
                  >
                    <span className="nb-setup-step-number" aria-hidden="true">
                      {complete || index + 1 < step ? (
                        <Check size={12} />
                      ) : (
                        index + 1
                      )}
                    </span>
                    <span>{label}</span>
                  </li>
                ))}
              </ol>
            </nav>
          )}
          <main className="nb-setup-content" aria-busy={step === null}>
            <h1 ref={heading} tabIndex={-1}>
              {title}
            </h1>
            {subtitle && <p className="nb-setup-subtitle">{subtitle}</p>}
            {children}
          </main>
          {actions && <footer className="nb-setup-footer">{actions}</footer>}
        </div>
      </div>
    );
  }

  return (
    <div className="nyxbot-onboarding" lang={i18n.resolvedLanguage}>
      <div className="nb-shell">
        <nav className="nb-tools" aria-label={t("title")}>
          <select
            aria-label={t("language")}
            value={i18n.resolvedLanguage}
            onChange={(event) => void i18n.changeLanguage(event.target.value)}
          >
            <option value="en">EN</option>
            <option value="zh-CN">简体中文</option>
          </select>
          <button
            type="button"
            onClick={toggleTheme}
            aria-label={t("theme")}
            title={t("theme")}
          >
            {theme === "dark" ? <Sun size={16} /> : <Moon size={16} />}
          </button>
        </nav>
        <header className="nb-header">
          <span className="nb-logo">
            <img src="/nyxid-icon-white.svg" alt="Nyxbot" />
          </span>
          <h1 ref={heading} tabIndex={-1}>
            {title}
          </h1>
          <p>{subtitle ?? t("step", { step })}</p>
          {step !== null && (
            <div
              className="nb-progress"
              role="progressbar"
              aria-label={t("title")}
              aria-valuemin={0}
              aria-valuemax={3}
              aria-valuenow={step}
            >
              <span style={{ width: `${(step / 3) * 100}%` }} />
            </div>
          )}
        </header>
        <main className="nb-content">{children}</main>
        <footer className="nb-footer">{actions}</footer>
      </div>
    </div>
  );
}

export function OnboardingLoading() {
  const { t } = useTranslation();
  return (
    <OnboardingShell
      title={t("loading")}
      step={null}
      variant="setup"
      actions={null}
    >
      <div className="nb-setup-loading" role="status">
        <LoaderCircle
          className="animate-spin motion-reduce:animate-none"
          size={20}
          aria-hidden="true"
        />
        <p>{t("checkingSetup")}</p>
      </div>
    </OnboardingShell>
  );
}

export function BackButton({
  onClick,
  disabled = false,
}: {
  readonly onClick: () => void;
  readonly disabled?: boolean;
}) {
  const { t } = useTranslation();
  return (
    <Button
      type="button"
      className="nb-secondary"
      onClick={onClick}
      disabled={disabled}
    >
      {t("back")}
    </Button>
  );
}
