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
        <h1 className="max-w-[700px] text-center font-serif text-[32px] leading-tight text-white md:text-5xl lg:text-6xl">
          {t("hero.eyebrow")}
        </h1>

        <p className="mt-6 max-w-2xl text-center text-lg leading-relaxed text-gray-300">
          {t("hero.title")}
        </p>

        <p className="mt-4 max-w-2xl text-center text-base leading-relaxed text-gray-400">
          {t("hero.subtitle")}
        </p>

        <a
          href="#waitlist"
          className="mt-10 inline-flex items-center gap-2 rounded-xl bg-primary px-8 py-4 text-lg font-semibold text-white transition-all hover:bg-void-400 hover:shadow-lg hover:shadow-primary/25"
        >
          {t("hero.cta")}
          <svg
            width="20"
            height="20"
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

        <p className="mt-4 text-sm text-gray-500">{t("hero.ctaSubtext")}</p>
      </div>
    </section>
  );
}
