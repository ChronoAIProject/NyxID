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

export function LandingPage() {
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
