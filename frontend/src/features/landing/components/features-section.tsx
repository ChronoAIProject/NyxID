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
    <path d="M22 12h-4l-3 9L9 3l-3 9H2" />
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
    <path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z" />
    <circle cx="12" cy="12" r="3" />
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
    <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
  </svg>,
  <svg
    key="3"
    width="24"
    height="24"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
  >
    <rect x="3" y="11" width="18" height="11" rx="2" ry="2" />
    <path d="M7 11V7a5 5 0 0110 0v4" />
  </svg>,
];

const featureKeys = [
  "pushApprovals",
  "seeBeforeApprove",
  "securityByDesign",
  "manageRevoke",
] as const;

export function FeaturesSection() {
  const { ref, inView } = useInView();
  const { t } = useTranslation();

  return (
    <section className="px-6 pt-8 pb-20" ref={ref}>
      <div className="mx-auto max-w-6xl">
        <h2 className="mb-4 text-center text-3xl font-bold tracking-tight text-foreground md:text-4xl">
          {t("features.heading")}
        </h2>
        <p className="mx-auto mb-12 max-w-2xl text-center text-muted-foreground">
          {t("features.subheading")}
        </p>

        <div className="grid gap-6 md:grid-cols-2">
          {featureKeys.map((key, i) => (
            <div
              key={key}
              className={`group rounded-xl border border-border/50 bg-card p-6 transition-colors hover:border-white/[0.15] ${
                inView ? "animate-fade-up" : "opacity-0"
              }`}
              style={{ animationDelay: `${i * 100}ms` }}
            >
              <div className="mb-4 flex h-12 w-12 items-center justify-center rounded-xl bg-nyx-secondary-400/10 text-nyx-secondary-400">
                {icons[i]}
              </div>
              <h3 className="mb-2 text-lg font-semibold text-foreground">
                {t(`features.${key}.title`)}
              </h3>
              <p className="leading-relaxed text-muted-foreground">
                {t(`features.${key}.description`)}
              </p>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}
