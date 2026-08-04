import { useTranslation } from "react-i18next";
import { useInView } from "../hooks/use-in-view";

const icons = [
  <svg
    key="0"
    width="24"
    height="24"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
  >
    <polyline points="16 18 22 12 16 6" />
    <polyline points="8 6 2 12 8 18" />
  </svg>,
  <svg
    key="1"
    width="24"
    height="24"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
  >
    <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
  </svg>,
  <svg
    key="2"
    width="24"
    height="24"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
  >
    <path d="M17 21v-2a4 4 0 00-4-4H5a4 4 0 00-4 4v2" />
    <circle cx="9" cy="7" r="4" />
    <path d="M23 21v-2a4 4 0 00-3-3.87" />
    <path d="M16 3.13a4 4 0 010 7.75" />
  </svg>,
];

const segmentKeys = ["developers", "security", "founders"] as const;

export function WhoItsFor() {
  const { ref, inView } = useInView();
  const { t } = useTranslation();

  return (
    <section className="px-6 py-20" ref={ref}>
      <div className="mx-auto max-w-6xl">
        <h2 className="mb-4 text-center text-3xl font-bold tracking-tight text-foreground md:text-4xl">
          {t("whoItsFor.heading")}
        </h2>
        <p className="mx-auto mb-12 max-w-xl text-center text-muted-foreground">
          {t("whoItsFor.subheading")}
        </p>

        <div className="grid gap-6 md:grid-cols-3">
          {segmentKeys.map((key, i) => (
            <div
              key={key}
              className={`rounded-xl border border-border/50 bg-card p-6 transition-colors hover:border-white/[0.15] ${
                inView ? "animate-fade-up" : "opacity-0"
              }`}
              style={{ animationDelay: `${i * 120}ms` }}
            >
              <div className="mb-4 flex h-12 w-12 items-center justify-center rounded-xl bg-nyx-secondary-400/10 text-nyx-secondary-400">
                {icons[i]}
              </div>
              <h3 className="mb-2 text-lg font-semibold text-foreground">
                {t(`whoItsFor.${key}.title`)}
              </h3>
              <p className="leading-relaxed text-muted-foreground">
                {t(`whoItsFor.${key}.description`)}
              </p>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}
