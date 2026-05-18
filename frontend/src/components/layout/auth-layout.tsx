import { useState, useEffect } from "react";
import { Link, Outlet } from "@tanstack/react-router";

const AUTH_IMAGES = [
  "/auth-hero.png",
  "/auth-hero-2.png",
  "/auth-hero-3.png",
  "/auth-hero-4.png",
];

const INTERVAL_MS = 6000;
const FADE_MS = 1200;

export function AuthLayout() {
  const [activeIndex, setActiveIndex] = useState(0);

  useEffect(() => {
    const timer = setInterval(() => {
      setActiveIndex((prev) => (prev + 1) % AUTH_IMAGES.length);
    }, INTERVAL_MS);
    return () => clearInterval(timer);
  }, []);

  return (
    <div className="flex h-dvh overflow-hidden bg-background">
      {/* Left — form column */}
      <div
        className="relative flex w-full shrink-0 flex-col border-r border-border bg-card lg:w-[calc(40%-20px)]"
        style={{
          paddingTop: "max(0px, var(--sat))",
          paddingBottom: "max(0px, var(--sab))",
          paddingLeft: "max(0px, var(--sal))",
        }}
      >
        {/* Logo — top-left */}
        <div className="flex shrink-0 items-center px-10 pt-8">
          <Link to="/login">
            <img
              src="/nyxid-coloured-logo.svg"
              alt="NyxID"
              className="h-7 w-auto"
            />
          </Link>
        </div>

        {/* Form area — vertically centered */}
        <div className="flex flex-1 items-center justify-center overflow-y-auto px-10 py-10">
          <div className="w-full max-w-[400px]">
            <Outlet />
          </div>
        </div>

        {/* Footer — bottom-left */}
        <div className="shrink-0 px-10 pb-8">
          <p className="text-center text-[11px] leading-relaxed text-muted-foreground">
            By continuing, you agree to NyxID&apos;s{" "}
            <Link to={"/terms" as string} className="text-muted-foreground underline underline-offset-2 hover:text-foreground">Terms of Service</Link>{" "}
            and
            <br />
            <Link to="/privacy" className="text-muted-foreground underline underline-offset-2 hover:text-foreground">Privacy Policy</Link>
            , and to receive periodic emails with updates.
          </p>
        </div>
      </div>

      {/* Right — hero image carousel with fade (hidden on mobile) */}
      <div className="relative hidden overflow-hidden lg:block lg:w-[calc(60%+20px)]">
        {AUTH_IMAGES.map((src, i) => (
          <img
            key={src}
            src={src}
            alt=""
            className="absolute inset-0 h-full w-full object-cover object-bottom"
            style={{
              transform: "scale(1.15)",
              transformOrigin: "center bottom",
              opacity: i === activeIndex ? 1 : 0,
              transition: `opacity ${FADE_MS}ms ease-in-out`,
            }}
          />
        ))}
      </div>
    </div>
  );
}
