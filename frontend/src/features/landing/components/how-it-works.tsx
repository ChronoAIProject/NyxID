import { useRef } from "react";
import { useTranslation } from "react-i18next";
import { useScroll } from "../hooks/use-scroll";

const stepNumbers = ["01", "02", "03"];
const stepKeys = ["step1", "step2", "step3"] as const;

export function HowItWorks() {
  const { t } = useTranslation();
  const sectionRef = useRef<HTMLDivElement>(null);
  const lineRef = useRef<HTMLDivElement>(null);
  const glowRef = useRef<HTMLDivElement>(null);
  const trackRef = useRef<HTMLDivElement>(null);
  const stepRefs = useRef<(HTMLDivElement | null)[]>([]);

  useScroll((_scrollY, vh) => {
    if (!sectionRef.current || !lineRef.current || !glowRef.current) return;

    const stepsContainer = lineRef.current.parentElement;
    if (!stepsContainer) return;
    const rect = stepsContainer.getBoundingClientRect();

    const start = rect.top;
    const end = rect.bottom + vh * 0.3;
    const p = Math.max(0, Math.min(1, (vh - start) / (end - start)));

    // Compute anchor points from actual step card positions
    const firstStep = stepRefs.current[0];
    const lastStep = stepRefs.current[stepKeys.length - 1];
    let minPx = 0;
    let maxPx = rect.height;
    if (firstStep) {
      const r = firstStep.getBoundingClientRect();
      minPx = r.top + r.height / 2 - rect.top;
    }
    if (lastStep) {
      const r = lastStep.getBoundingClientRect();
      maxPx = r.top + r.height / 2 - rect.top;
    }

    // Fill in pixels, clamped between first and last step centers
    const range = maxPx - minPx;
    const fillPx = Math.max(0, Math.min(range, p * range));

    // Position the static track between first and last step centers
    if (trackRef.current) {
      trackRef.current.style.top = `${minPx}px`;
      trackRef.current.style.height = `${range}px`;
    }

    lineRef.current.style.top = `${minPx}px`;
    lineRef.current.style.height = `${fillPx}px`;

    glowRef.current.style.top = `${minPx + fillPx}px`;
    glowRef.current.style.opacity = String(
      p > 0.05 && fillPx < range ? 1 : 0,
    );

    stepRefs.current.forEach((el, i) => {
      if (!el) return;
      const threshold = (i + 0.5) / stepKeys.length;
      if (p >= threshold) {
        el.classList.add(
          "border-nyx-secondary-400/50",
          "shadow-lg",
          "shadow-nyx-secondary-400/20",
        );
        el.classList.remove("border-border/50");
      } else {
        el.classList.remove(
          "border-nyx-secondary-400/50",
          "shadow-lg",
          "shadow-nyx-secondary-400/20",
        );
        el.classList.add("border-border/50");
      }
    });
  });

  return (
    <section className="px-6 py-20">
      <div className="mx-auto max-w-4xl" ref={sectionRef}>
        <h2 className="mb-4 text-center text-3xl font-bold tracking-tight text-foreground md:text-4xl">
          {t("howItWorks.heading")}
        </h2>
        <p className="mx-auto mb-12 max-w-xl text-center text-muted-foreground">
          {t("howItWorks.subheading")}
        </p>

        <div className="relative space-y-12">
          {/* Track — positioned dynamically between first and last step centers */}
          <div
            ref={trackRef}
            className="absolute left-[39px] w-[2px] bg-nyx-secondary-400/10"
            style={{ top: 0, height: 0 }}
          />

          {/* Fill */}
          <div
            ref={lineRef}
            className="absolute left-[39px] w-[2px] bg-gradient-to-b from-nyx-secondary-400 via-nyx-300 to-nyx-secondary-400"
            style={{ top: 0, height: 0, transition: "height 0.05s linear" }}
          />

          {/* Glow dot */}
          <div
            ref={glowRef}
            className="absolute left-[33px] h-[14px] w-[14px] -translate-y-1/2 rounded-full"
            style={{
              top: "0%",
              opacity: 0,
              background:
                "radial-gradient(circle, rgba(166,114,251,0.8) 0%, rgba(166,114,251,0.3) 50%, transparent 70%)",
              boxShadow:
                "0 0 12px rgba(166,114,251,0.6), 0 0 30px rgba(166,114,251,0.3)",
              transition: "top 0.05s linear, opacity 0.3s ease",
            }}
          />

          {stepKeys.map((key, i) => (
            <div
              key={key}
              className="relative flex gap-8"
            >
              <div
                ref={(el) => {
                  stepRefs.current[i] = el;
                }}
                className="flex h-20 w-20 shrink-0 items-center justify-center rounded-xl border border-border/50 bg-card font-mono text-2xl font-medium text-nyx-secondary-400 transition-all duration-500"
              >
                {stepNumbers[i]}
              </div>
              <div className="pt-2">
                <h3 className="mb-2 text-lg font-semibold text-foreground">
                  {t(`howItWorks.${key}.title`)}
                </h3>
                <p className="leading-relaxed text-muted-foreground">
                  {t(`howItWorks.${key}.description`)}
                </p>
              </div>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}
