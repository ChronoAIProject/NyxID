import { useState, useRef, useEffect, useCallback } from "react";
import { useTranslation } from "react-i18next";

const languages = [
  { code: "en", label: "EN" },
  { code: "zh-CN", label: "简体" },
  { code: "zh-TW", label: "繁體" },
];

export function LanguageSwitcher() {
  const { i18n } = useTranslation();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  const current =
    languages.find((l) => l.code === i18n.language) ?? languages[0];

  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node))
        setOpen(false);
    };
    const esc = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", esc);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", esc);
    };
  }, [open]);

  const select = useCallback(
    (code: string) => {
      i18n.changeLanguage(code);
      setOpen(false);
    },
    [i18n],
  );

  return (
    <div ref={ref} className="relative">
      <button
        onClick={() => setOpen((o) => !o)}
        className="flex h-9 items-center gap-1 rounded-full bg-white/[0.04] px-2.5 text-xs text-muted-foreground transition-colors hover:text-foreground"
        aria-label="Change language"
      >
        <svg
          width="15"
          height="15"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.8"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <circle cx="12" cy="12" r="10" />
          <path d="M2 12h20" />
          <path d="M12 2a15.3 15.3 0 014 10 15.3 15.3 0 01-4 10 15.3 15.3 0 01-4-10 15.3 15.3 0 014-10z" />
        </svg>
        <span className="text-foreground">{current?.label ?? "EN"}</span>
      </button>

      {open && (
        <div className="absolute left-0 top-full z-50 mt-1.5 min-w-[80px] overflow-hidden rounded-xl border border-border/50 bg-card shadow-xl shadow-black/40">
          {languages.map((lang) => (
            <button
              key={lang.code}
              onClick={() => select(lang.code)}
              className={`flex w-full items-center px-4 py-2 text-xs transition-colors ${
                i18n.language === lang.code
                  ? "text-nyx-secondary-400"
                  : "text-muted-foreground hover:bg-white/[0.06] hover:text-foreground"
              }`}
            >
              {lang.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
