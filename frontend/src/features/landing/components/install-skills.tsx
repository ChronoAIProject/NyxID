import { useState } from "react";
import { useTranslation } from "react-i18next";

const INSTALL_COMMAND =
  "Install nyx skills from https://github.com/ChronoAIProject/NyxID/blob/main/skills/INSTALL.md";

export function InstallSkills() {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(INSTALL_COMMAND);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // Clipboard API requires HTTPS or localhost; user can still select+copy manually.
    }
  };

  return (
    <section className="border-y border-landing-border-subtle bg-landing-surface/40 px-6 py-20">
      <div className="mx-auto max-w-3xl text-center">
        <h2 className="font-serif text-3xl text-white md:text-4xl">
          {t("install.heading")}
        </h2>
        <p className="mx-auto mt-4 max-w-2xl text-base leading-relaxed text-gray-400">
          {t("install.subheading")}
        </p>

        <div className="mt-10 flex flex-col items-stretch overflow-hidden rounded-xl border border-primary/30 bg-black/50 backdrop-blur-md sm:flex-row">
          <input
            readOnly
            value={INSTALL_COMMAND}
            onFocus={(e) => e.currentTarget.select()}
            aria-label={t("install.commandLabel")}
            className="flex-1 truncate bg-transparent px-5 py-4 text-left font-mono text-sm text-gray-200 outline-none"
          />
          <button
            type="button"
            onClick={handleCopy}
            className="flex items-center justify-center gap-2 border-t border-primary/30 bg-primary/15 px-6 py-4 text-sm font-semibold text-white transition-colors hover:bg-primary/30 sm:border-l sm:border-t-0"
          >
            {copied ? (
              <>
                <svg
                  width="16"
                  height="16"
                  viewBox="0 0 20 20"
                  fill="none"
                  aria-hidden="true"
                >
                  <path
                    d="M5 10l3 3 7-7"
                    stroke="currentColor"
                    strokeWidth="2"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  />
                </svg>
                {t("install.copied")}
              </>
            ) : (
              <>
                <svg
                  width="16"
                  height="16"
                  viewBox="0 0 20 20"
                  fill="none"
                  aria-hidden="true"
                >
                  <rect
                    x="6"
                    y="6"
                    width="11"
                    height="11"
                    rx="2"
                    stroke="currentColor"
                    strokeWidth="2"
                  />
                  <path
                    d="M4 14V5a2 2 0 012-2h9"
                    stroke="currentColor"
                    strokeWidth="2"
                    strokeLinecap="round"
                  />
                </svg>
                {t("install.copy")}
              </>
            )}
          </button>
        </div>

        <p className="mt-5 text-sm text-gray-500">{t("install.helper")}</p>
      </div>
    </section>
  );
}
