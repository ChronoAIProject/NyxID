import "./landing.css";
import "./i18n";

import { LandingNavbar } from "./components/landing-navbar";
import { Hero } from "./components/hero";
import { FeaturesSection } from "./components/features-section";
import { WhyNyx } from "./components/why-nyx";
import { HowItWorks } from "./components/how-it-works";
import { WhoItsFor } from "./components/who-its-for";
import { AppCarousel } from "./components/app-carousel";
import { WaitlistForm } from "./components/waitlist-form";
import { ScrollFab } from "./components/scroll-fab";
import { LandingFooter } from "./components/landing-footer";

export function LandingPage() {
  return (
    <div className="min-h-screen overflow-x-hidden bg-landing-bg">
      <LandingNavbar />
      <ScrollFab />
      <main>
        <Hero />
        <FeaturesSection />
        <WhyNyx />
        <HowItWorks />
        <WhoItsFor />
        <AppCarousel />
        <WaitlistForm />
      </main>
      <LandingFooter />
    </div>
  );
}
