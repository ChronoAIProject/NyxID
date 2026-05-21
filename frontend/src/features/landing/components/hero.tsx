import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

const UNICORN_STUDIO_URL =
  "https://cdn.jsdelivr.net/gh/hiunicornstudio/unicornstudio.js@v2.1.4/dist/unicornStudio.umd.js";

export function Hero() {
  const { t } = useTranslation();
  const destroyRef = useRef<(() => void) | null>(null);
  const [bgReady, setBgReady] = useState(false);

  useEffect(() => {
    let cancelled = false;

    // Defer US loading until browser is idle / LCP has fired
    const startLoad = () => {
      if (cancelled) return;

      const tryInit = () => {
        if (cancelled) return;
        // @ts-expect-error global UnicornStudio
        const US = window.UnicornStudio;
        if (!US) {
          setTimeout(tryInit, 300);
          return;
        }
        US.init()
          .then((scenes: { destroy: () => void }[]) => {
            const scene = scenes?.[0];
            if (!cancelled && scene) {
              destroyRef.current = () => scene.destroy();
            }
            if (!cancelled) setBgReady(true);
          })
          .catch(() => {
            if (!cancelled) setBgReady(true);
          });
      };

      const existing = document.querySelector(
        `script[src="${UNICORN_STUDIO_URL}"]`,
      );

      if (existing) {
        setTimeout(tryInit, 300);
      } else {
        const script = document.createElement("script");
        script.src = UNICORN_STUDIO_URL;
        script.onload = () => setTimeout(tryInit, 500);
        document.head.appendChild(script);
      }
    };

    // Wait for idle or 2s max — keeps main thread free for LCP
    if ("requestIdleCallback" in window) {
      requestIdleCallback(startLoad, { timeout: 2000 });
    } else {
      setTimeout(startLoad, 2000);
    }

    // Fallback: fade in after 4s even if US never initializes
    const fallback = setTimeout(() => {
      if (!cancelled) setBgReady(true);
    }, 4000);

    return () => {
      cancelled = true;
      clearTimeout(fallback);
      destroyRef.current?.();
      destroyRef.current = null;
    };
  }, []);

  return (
    <section
      className="relative flex min-h-screen flex-col items-center justify-center px-6 pt-20 pb-[100px]"
      style={{ isolation: "isolate", marginBottom: -2 }}
    >
      {/* Unicorn Studio background — fades in after LCP */}
      <div
        data-us-project="sqyz3r4WjVjdLal6IHvt"
        className={`absolute inset-0 z-0 transition-opacity duration-[2000ms] ease-out ${bgReady ? "opacity-100" : "opacity-0"}`}
      />

      {/* Bottom bar to cover watermark — taller on mobile */}
      <div className="absolute bottom-0 left-0 right-0 z-10 h-[200px] bg-gradient-to-t from-landing-bg from-[67%] to-transparent md:h-[120px]" />

      {/* Content */}
      <div className="relative z-10 mt-[260px] flex flex-col items-center md:mt-[190px]">
        <img
          src="/nyxid-wordmark.svg"
          alt="NyxID"
          className="mb-7 h-9 w-auto drop-shadow-[0_0_24px_rgba(90,42,241,0.45)] md:h-10"
        />
        <h1
          className="max-w-[640px] text-center text-[28px] font-bold leading-[1.1] tracking-tight text-foreground sm:text-4xl lg:text-5xl"
          style={{ letterSpacing: "-0.03em" }}
        >
          {t("hero.eyebrow")}
        </h1>

        <p className="mt-5 max-w-xl text-center text-[15px] leading-relaxed text-muted-foreground sm:text-base">
          {t("hero.title")}
        </p>

        <p className="mt-3 max-w-xl text-center text-[13px] leading-relaxed text-muted-foreground/80 sm:text-sm">
          {t("hero.subtitle")}
        </p>

        <div className="mt-8 flex flex-col items-center gap-3 sm:flex-row">
          <a
            href={`/register${typeof window !== "undefined" ? window.location.search : ""}`}
            className="inline-flex items-center gap-2 rounded-lg nyx-gradient-vivid px-6 py-3 text-sm font-medium text-white shadow-[0_2px_12px_rgba(90,42,241,0.25)] transition-all hover:shadow-[0_4px_20px_rgba(90,42,241,0.35)] hover:brightness-110"
          >
            {t("hero.ctaRegister")}
            <svg
              width="16"
              height="16"
              viewBox="0 0 20 20"
              fill="none"
              aria-hidden="true"
            >
              <path
                d="M4 10h12m0 0l-4-4m4 4l-4 4"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
          </a>
          <a
            href="#beta"
            className="inline-flex items-center gap-2 rounded-lg border border-white/[0.08] px-6 py-3 text-sm font-medium text-foreground transition-colors hover:border-white/[0.15] hover:bg-white/[0.03]"
          >
            {t("hero.ctaEarlyAccess")}
          </a>
        </div>

        <p className="mt-4 text-[11px] text-text-tertiary">{t("hero.ctaSubtext")}</p>
      </div>
    </section>
  );
}
