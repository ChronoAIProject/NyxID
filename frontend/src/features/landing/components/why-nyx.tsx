import { useRef } from "react";
import { useTranslation } from "react-i18next";
import { useInView } from "../hooks/use-in-view";
import { useScroll } from "../hooks/use-scroll";

const clouds: {
  src: string;
  width: number;
  imgOpacity: number;
  z: number;
  anchor: Record<string, string>;
  startX: number;
  startY: number;
  endX: number;
  endY: number;
  rotate?: number;
}[] = [
  // FROM LEFT
  {
    src: "/landing/cloud-1.webp",
    width: 380,
    imgOpacity: 0.5,
    z: 5,
    anchor: { top: "10%", left: "10%" },
    startX: -500,
    startY: 20,
    endX: 200,
    endY: -10,
    rotate: 180,
  },
  {
    src: "/landing/cloud-3.webp",
    width: 300,
    imgOpacity: 0.4,
    z: 15,
    anchor: { top: "49%", left: "5%" },
    startX: -600,
    startY: -30,
    endX: 1400,
    endY: 10,
  },
  {
    src: "/landing/cloud-2.webp",
    width: 350,
    imgOpacity: 0.45,
    z: 15,
    anchor: { bottom: "9%", left: "8%" },
    startX: -540,
    startY: 48,
    endX: 600,
    endY: -24,
  },
  // FROM RIGHT
  {
    src: "/landing/cloud-4.webp",
    width: 380,
    imgOpacity: 0.45,
    z: 5,
    anchor: { top: "16%", right: "10%" },
    startX: 500,
    startY: 15,
    endX: -400,
    endY: -10,
  },
  {
    src: "/landing/cloud-5.webp",
    width: 400,
    imgOpacity: 0.35,
    z: 15,
    anchor: { top: "69%", right: "5%" },
    startX: 700,
    startY: 0,
    endX: -1500,
    endY: 15,
  },
  {
    src: "/landing/cloud-6.webp",
    width: 340,
    imgOpacity: 0.5,
    z: 5,
    anchor: { bottom: "-8%", right: "8%" },
    startX: 400,
    startY: 50,
    endX: -250,
    endY: -30,
  },
];

export function WhyNyx() {
  const { ref, inView } = useInView();
  const { t } = useTranslation();
  const sectionRef = useRef<HTMLDivElement>(null);
  const moonRef = useRef<HTMLDivElement>(null);
  const cloudRefs = useRef<(HTMLDivElement | null)[]>([]);

  useScroll((_scrollY, vh) => {
    if (!sectionRef.current) return;
    const rect = sectionRef.current.getBoundingClientRect();
    const p = Math.max(
      0,
      Math.min(1, 1 - (rect.top + rect.height) / (vh * 1.5 + rect.height)),
    );

    if (moonRef.current) {
      moonRef.current.style.transform = `translate3d(${p * 120}px, ${(1 - p) * 60}px, 0)`;
      moonRef.current.style.opacity = String(Math.min(1, p * 2));
    }

    cloudRefs.current.forEach((el, i) => {
      if (!el) return;
      const c = clouds[i]!;
      const x = c.startX + (c.endX - c.startX) * p;
      const y = c.startY + (c.endY - c.startY) * p;
      el.style.transform = `translate3d(${x}px, ${y}px, 0)`;
      el.style.opacity = String(Math.min(1, p * 2));
    });
  });

  return (
    <section
      ref={sectionRef}
      className="relative overflow-x-clip md:mb-[-100vh]"
      style={{ height: "200vh" }}
    >
      <div
        className="sticky top-0 overflow-hidden px-6 py-24"
        style={{
          minHeight: "100vh",
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
        }}
      >
        {/* moon */}
        <div
          ref={moonRef}
          className="pointer-events-none absolute will-change-transform"
          style={{ top: "18%", left: "5%", opacity: 0 }}
        >
          <div
            className="absolute glow-pulse"
            style={{
              width: 400,
              height: 400,
              top: "10%",
              left: "5%",
              background:
                "radial-gradient(circle, rgba(139,92,246,0.14) 0%, rgba(139,92,246,0.04) 45%, transparent 70%)",
              filter: "blur(20px)",
            }}
          />
          <img
            src="/landing/moon.webp"
            alt=""
            className="relative block opacity-60"
            style={{ width: 500, height: "auto" }}
            draggable={false}
            loading="lazy"
          />
        </div>

        {/* clouds */}
        {clouds.map((c, i) => (
          <div
            key={i}
            ref={(el) => {
              cloudRefs.current[i] = el;
            }}
            className="pointer-events-none absolute will-change-transform"
            style={{ opacity: 0, zIndex: c.z, ...c.anchor }}
          >
            <img
              src={c.src}
              alt=""
              className="block"
              style={{
                width: c.width,
                height: "auto",
                opacity: c.imgOpacity,
                transform: c.rotate ? `rotate(${c.rotate}deg)` : undefined,
              }}
              draggable={false}
              loading="lazy"
            />
          </div>
        ))}

        {/* card (z-10) */}
        <div
          ref={ref}
          className={`relative mx-auto max-w-3xl ${inView ? "animate-fade-up" : "opacity-0"}`}
          style={{ zIndex: 10 }}
        >
          <div className="rounded-2xl border border-landing-border-subtle bg-landing-surface/90 p-10 backdrop-blur-sm md:p-14">
            <p className="mb-6 font-mono text-xs uppercase tracking-widest text-primary">
              {t("whyNyx.label")}
            </p>

            <blockquote className="mb-8 font-serif text-2xl leading-relaxed text-white md:text-3xl">
              {t("whyNyx.quote")}
            </blockquote>

            <div className="space-y-4 leading-relaxed text-gray-300">
              <p>{t("whyNyx.para1")}</p>
              <p>{t("whyNyx.para2")}</p>
              <p className="font-medium text-void-200">
                {t("whyNyx.para3")}
              </p>
            </div>
          </div>
        </div>

        {/* edge fade */}
        <div
          className="pointer-events-none absolute inset-x-0 top-0 h-24 bg-gradient-to-b from-landing-bg to-transparent"
          style={{ zIndex: 30 }}
        />
        <div
          className="pointer-events-none absolute inset-x-0 bottom-0 h-24 bg-gradient-to-t from-landing-bg to-transparent"
          style={{ zIndex: 30 }}
        />
      </div>
    </section>
  );
}
