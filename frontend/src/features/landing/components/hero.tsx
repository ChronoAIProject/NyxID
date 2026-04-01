import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";

const UNICORN_STUDIO_URL =
  "https://cdn.jsdelivr.net/gh/hiunicornstudio/unicornstudio.js@v2.1.4/dist/unicornStudio.umd.js";

export function Hero() {
  const { t } = useTranslation();
  const destroyRef = useRef<(() => void) | null>(null);

  useEffect(() => {
    let cancelled = false;

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
        })
        .catch(() => {});
    };

    // Dynamically load Unicorn Studio script
    const existing = document.querySelector(
      `script[src="${UNICORN_STUDIO_URL}"]`,
    );

    if (existing) {
      // Script already loaded (e.g. navigated back), poll for readiness
      setTimeout(tryInit, 300);
    } else {
      const script = document.createElement("script");
      script.src = UNICORN_STUDIO_URL;
      script.onload = () => setTimeout(tryInit, 500);
      document.head.appendChild(script);
    }

    return () => {
      cancelled = true;
      destroyRef.current?.();
      destroyRef.current = null;
    };
  }, []);

  return (
    <section
      className="relative flex min-h-screen flex-col items-center justify-center px-6 pt-20 pb-[100px]"
      style={{ isolation: "isolate", marginBottom: -2 }}
    >
      {/* Unicorn Studio background */}
      <div
        data-us-project="sqyz3r4WjVjdLal6IHvt"
        className="absolute inset-0 z-0"
      />

      {/* Bottom bar to cover watermark */}
      <div className="absolute bottom-0 left-0 right-0 z-10 h-[120px] bg-gradient-to-t from-landing-bg from-[67%] to-transparent" />

      {/* Content */}
      <div className="relative z-10 mt-[190px] flex flex-col items-center">
        <h1 className="max-w-3xl text-center font-serif text-5xl leading-tight text-white md:text-6xl lg:text-7xl">
          {t("hero.title")}{" "}
          <span className="text-void-400">{t("hero.titleAccent")}</span>
        </h1>

        <p className="mt-6 max-w-2xl text-center text-lg leading-relaxed text-gray-300">
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
