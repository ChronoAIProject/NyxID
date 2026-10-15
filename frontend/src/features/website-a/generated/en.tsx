// Generated from approved A, SHA-256 5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33.
// Run: node scripts/website-a/generate.mjs <baseline.html>
import { lazy, useEffect } from "react";
import { useWebsite } from "../website-context";
import WebsiteHeader from "./en/WebsiteHeader";
import WebsiteFooter from "./en/WebsiteFooter";
const HomePage = lazy(() => import("./en/HomePage"));
const NyxbotPage = lazy(() => import("./en/NyxbotPage"));
const SolutionsPage = lazy(() => import("./en/SolutionsPage"));
const HowItWorksPage = lazy(() => import("./en/HowItWorksPage"));
const SecurityPage = lazy(() => import("./en/SecurityPage"));
const AgentsPage = lazy(() => import("./en/AgentsPage"));
const PricingPage = lazy(() => import("./en/PricingPage"));
const IntegrationsPage = lazy(() => import("./en/IntegrationsPage"));
const BlogPage = lazy(() => import("./en/BlogPage"));
const ContactPage = lazy(() => import("./en/ContactPage"));
const NotfoundPage = lazy(() => import("./en/NotfoundPage"));
const SolutionsRestaurantManagerPage = lazy(
  () => import("./en/SolutionsRestaurantManagerPage"),
);
const SolutionsHelpDeskPage = lazy(() => import("./en/SolutionsHelpDeskPage"));
const SolutionsAppointmentBookerPage = lazy(
  () => import("./en/SolutionsAppointmentBookerPage"),
);
const SolutionsCallbackQueuePage = lazy(
  () => import("./en/SolutionsCallbackQueuePage"),
);
const SolutionsStockAnalystPage = lazy(
  () => import("./en/SolutionsStockAnalystPage"),
);
const SolutionsNewsAggregatorPage = lazy(
  () => import("./en/SolutionsNewsAggregatorPage"),
);
const SolutionsDeskResearcherPage = lazy(
  () => import("./en/SolutionsDeskResearcherPage"),
);
const SolutionsXLeadCatcherPage = lazy(
  () => import("./en/SolutionsXLeadCatcherPage"),
);
const SolutionsBackOfficeClerkPage = lazy(
  () => import("./en/SolutionsBackOfficeClerkPage"),
);
const BlogWhatIsNyxidPage = lazy(() => import("./en/BlogWhatIsNyxidPage"));
const BlogLunaBistroPage = lazy(() => import("./en/BlogLunaBistroPage"));
const BlogNeverSharePasswordsPage = lazy(
  () => import("./en/BlogNeverSharePasswordsPage"),
);
const BlogFiveJobsPage = lazy(() => import("./en/BlogFiveJobsPage"));
const BlogScopedKeysPage = lazy(() => import("./en/BlogScopedKeysPage"));
const BlogXDmsLivePage = lazy(() => import("./en/BlogXDmsLivePage"));
const pages = {
  "/nav": WebsiteHeader,
  "/footer": WebsiteFooter,
  "/": HomePage,
  "/nyxbot": NyxbotPage,
  "/solutions": SolutionsPage,
  "/how-it-works": HowItWorksPage,
  "/security": SecurityPage,
  "/agents": AgentsPage,
  "/pricing": PricingPage,
  "/integrations": IntegrationsPage,
  "/blog": BlogPage,
  "/contact": ContactPage,
  "/404": NotfoundPage,
  "/solutions/restaurant-manager": SolutionsRestaurantManagerPage,
  "/solutions/help-desk": SolutionsHelpDeskPage,
  "/solutions/appointment-booker": SolutionsAppointmentBookerPage,
  "/solutions/callback-queue": SolutionsCallbackQueuePage,
  "/solutions/stock-analyst": SolutionsStockAnalystPage,
  "/solutions/news-aggregator": SolutionsNewsAggregatorPage,
  "/solutions/desk-researcher": SolutionsDeskResearcherPage,
  "/solutions/x-lead-catcher": SolutionsXLeadCatcherPage,
  "/solutions/back-office-clerk": SolutionsBackOfficeClerkPage,
  "/blog/what-is-nyxid": BlogWhatIsNyxidPage,
  "/blog/luna-bistro": BlogLunaBistroPage,
  "/blog/never-share-passwords": BlogNeverSharePasswordsPage,
  "/blog/five-jobs": BlogFiveJobsPage,
  "/blog/scoped-keys": BlogScopedKeysPage,
  "/blog/x-dms-live": BlogXDmsLivePage,
};
const titles: Record<string, string> = {
  "/": "AI that does more than chat.",
  "/nyxbot": "Just ask. NyxBot handles it.",
  "/solutions": "What should your assistant do?",
  "/how-it-works": "How NyxID connects AI to your world, safely.",
  "/security": "Your passwords never reach the AI.",
  "/agents": "Already use an AI agent? Plug it into NyxID.",
  "/pricing": "Platform prices. No markup.",
  "/integrations": "Connect the tools you already use.",
  "/blog": "Guides, stories & updates",
  "/contact": "Talk to us",
  "/404": "Page not found",
  "/solutions/restaurant-manager": "Restaurant manager",
  "/solutions/help-desk": "24-hour help desk",
  "/solutions/appointment-booker": "Appointment booker",
  "/solutions/callback-queue": "Callback queue",
  "/solutions/stock-analyst": "Stock analyst",
  "/solutions/news-aggregator": "News aggregator",
  "/solutions/desk-researcher": "Desk researcher",
  "/solutions/x-lead-catcher": "X lead catcher",
  "/solutions/back-office-clerk": "Back-office clerk",
  "/blog/what-is-nyxid":
    "What is an “agent gateway”? A plain-English guide for business owners",
  "/blog/luna-bistro":
    "How a 40-seat bistro stopped missing late-night bookings",
  "/blog/never-share-passwords": "Why your AI should never see your passwords",
  "/blog/five-jobs": "Five repetitive jobs to hand off to an AI this month",
  "/blog/scoped-keys": "Scoped keys: give agents only what they need",
  "/blog/x-dms-live": "Product update: X (Twitter) DMs are live",
};
export default function WebsiteContent() {
  const s = useWebsite();
  const Page =
    (pages as Record<string, React.ElementType>)[s.route] ?? pages["/404"];
  useEffect(() => {
    document.title = (titles[s.route] ?? titles["/404"]) + " · NyxID";
  }, [s.route]);
  return (
    <>
      <nav className={"nav " + (s.menuOpen ? "open" : "")} id="nav">
        <WebsiteHeader />
      </nav>
      <main id="website-main" tabIndex={-1}>
        <Page />
      </main>
      <footer id="foot">
        <WebsiteFooter />
      </footer>
    </>
  );
}
