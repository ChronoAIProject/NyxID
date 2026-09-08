import { useEffect, useRef, type ReactNode } from "react";
import { CircleHelp, Moon, Sun } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { useApplyTheme, useResolvedTheme } from "@/hooks/use-theme";
import { useThemeStore } from "@/stores/theme-store";

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
}: {
  readonly title: string;
  readonly subtitle?: string;
  readonly step: 1 | 2 | 3;
  readonly children: ReactNode;
  readonly actions: ReactNode;
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
        </header>
        <main className="nb-content">{children}</main>
        <footer className="nb-footer">{actions}</footer>
      </div>
    </div>
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
