// Generated from approved A, SHA-256 5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33.
// Run: node scripts/website-a/generate.mjs <baseline.html>
import { lazy, Suspense } from "react";
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
};
export const titles: Record<string, string> = {
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
};
export const descriptions: Record<string, string> = {
  "/": "Hand the boring stuff to AI, so you can focus on what matters. Connect your apps, decide what it can touch, and let it get to work.",
  "/nyxbot":
    "The easy way to use NyxID. Tell it what you need in the app or on Telegram: it connects your services, sets what each assistant may do and automates the busywork. It isn’t a general chatbot; it works on your apps.",
  "/solutions":
    "Ready-made jobs for real businesses, with the right connections and safe permissions already set.",
  "/how-it-works":
    "Four layers, one rule: the AI only ever gets what you allow. Here’s what happens between “take this booking” and “done”.",
  "/security":
    "Logins stay encrypted inside NyxID. Each assistant gets a scoped key for only the actions you allow, risky steps wait for your OK, and every call is logged.",
  "/agents":
    "Claude, ChatGPT, Cursor, Codex or your own script: point it at NyxID and it gets scoped access to your services, AI models and machines, without ever holding a password. NyxID is open source: self-host it, or use ours.",
  "/pricing":
    "Every model and API call is billed at the provider’s published rate. NyxID adds nothing on top.",
  "/integrations":
    "145 apps and APIs across 30 categories. Connect each one with a secure sign-in link, then decide exactly what your AI may do with it.",
  "/blog": "Practical ideas for putting AI to work, safely.",
  "/contact":
    "Questions about bundles, self-hosting or your setup? Pick whichever is easiest.",
  "/404": "NyxID",
  "/solutions/restaurant-manager":
    "Takes table bookings in Telegram, checks your calendar, notes allergies and sends reminders.",
  "/solutions/help-desk":
    "Answers order, delivery and policy questions any time, and hands tricky ones to your team.",
  "/solutions/appointment-booker":
    "Offers open slots, books the right person, and handles reschedules with your OK.",
  "/solutions/callback-queue":
    "Works through your list of customers to call back, has the conversation with an AI voice, and logs the outcome.",
  "/solutions/stock-analyst":
    "Watches your watchlist, scans market chatter and briefs you in plain English. Reads, never trades.",
  "/solutions/news-aggregator":
    "Reads the sources you care about every morning and sends one tidy brief without duplicates.",
  "/solutions/desk-researcher":
    "Searches, reads and compares, then writes a finished report into your Google Docs.",
  "/solutions/x-lead-catcher":
    "Replies to product questions in your X DMs, qualifies the lead and adds it to your sheet.",
  "/solutions/back-office-clerk":
    "Logs in to supplier and booking portals that have no API, reorders stock, downloads invoices and files them, on your computer or a hosted VM.",
};
export const loaders = {
  "/": () => import("./en/HomePage"),
  "/nyxbot": () => import("./en/NyxbotPage"),
  "/solutions": () => import("./en/SolutionsPage"),
  "/how-it-works": () => import("./en/HowItWorksPage"),
  "/security": () => import("./en/SecurityPage"),
  "/agents": () => import("./en/AgentsPage"),
  "/pricing": () => import("./en/PricingPage"),
  "/integrations": () => import("./en/IntegrationsPage"),
  "/blog": () => import("./en/BlogPage"),
  "/contact": () => import("./en/ContactPage"),
  "/404": () => import("./en/NotfoundPage"),
  "/solutions/restaurant-manager": () =>
    import("./en/SolutionsRestaurantManagerPage"),
  "/solutions/help-desk": () => import("./en/SolutionsHelpDeskPage"),
  "/solutions/appointment-booker": () =>
    import("./en/SolutionsAppointmentBookerPage"),
  "/solutions/callback-queue": () => import("./en/SolutionsCallbackQueuePage"),
  "/solutions/stock-analyst": () => import("./en/SolutionsStockAnalystPage"),
  "/solutions/news-aggregator": () =>
    import("./en/SolutionsNewsAggregatorPage"),
  "/solutions/desk-researcher": () =>
    import("./en/SolutionsDeskResearcherPage"),
  "/solutions/x-lead-catcher": () => import("./en/SolutionsXLeadCatcherPage"),
  "/solutions/back-office-clerk": () =>
    import("./en/SolutionsBackOfficeClerkPage"),
};
export async function loadPage(route: string) {
  const load =
    (
      loaders as Record<string, () => Promise<{ default: React.ComponentType }>>
    )[route] ?? loaders["/404"];
  return (await load()).default;
}
export default function WebsiteContent() {
  const s = useWebsite();
  const Page =
    (pages as Record<string, React.ElementType>)[s.route] ?? pages["/404"];
  return (
    <Suspense fallback={<p role="status">Loading…</p>}>
      <Page />
    </Suspense>
  );
}
