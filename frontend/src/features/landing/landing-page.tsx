import { useEffect } from "react";

import "./landing.css";
import "./i18n";

import { LandingNavbar } from "./components/landing-navbar";
import { Hero } from "./components/hero";
import { InstallSkills } from "./components/install-skills";
import { FeaturesSection } from "./components/features-section";
import { WhyNyx } from "./components/why-nyx";
import { HowItWorks } from "./components/how-it-works";
import { WhoItsFor } from "./components/who-its-for";
import { AppCarousel } from "./components/app-carousel";
import { BetaAccess } from "./components/beta-access";
import { ScrollFab } from "./components/scroll-fab";
import { LandingFooter } from "./components/landing-footer";

// Stopgap: auto-applied for Reddit-referred users without an explicit ?code=.
// Operator must keep a matching invite code active in admin UI with high max_uses.
// Remove when proper attribution/campaign codes are wired up.
const REDDIT_DEFAULT_INVITE_CODE = "NYX-QJGT6NHN";

function injectRedditDefaultCode() {
  if (typeof window === "undefined") return;
  const params = new URLSearchParams(window.location.search);
  if (params.get("code")) return;

  let referrerHost = "";
  try {
    referrerHost = new URL(document.referrer).hostname.toLowerCase();
  } catch {
    referrerHost = "";
  }
  const fromReddit =
    /(^|\.)reddit\.com$|(^|\.)redd\.it$/.test(referrerHost) ||
    params.get("utm_source")?.toLowerCase() === "reddit";

  if (fromReddit) {
    params.set("code", REDDIT_DEFAULT_INVITE_CODE);
    window.history.replaceState(
      null,
      "",
      `${window.location.pathname}?${params.toString()}`,
    );
  }
}

export function LandingPage() {
  // Run synchronously before children render so LandingNavbar's loginHref
  // (computed at render time) sees the injected ?code=.
  injectRedditDefaultCode();

  useEffect(() => {
    const timer = setTimeout(() => {
      if (window.location.hash === "#beta") {
        document.getElementById("beta")?.scrollIntoView({ behavior: "smooth" });
      }
    }, 1000);
    return () => clearTimeout(timer);
  }, []);

  return (
    <div className="min-h-screen overflow-x-hidden bg-landing-bg">
      <LandingNavbar />
      <ScrollFab />
      <main>
        <Hero />
        <InstallSkills />
        <FeaturesSection />
        <WhyNyx />
        <HowItWorks />
        <WhoItsFor />
        <AppCarousel />
        <BetaAccess />
      </main>
      <LandingFooter />
    </div>
  );
}
