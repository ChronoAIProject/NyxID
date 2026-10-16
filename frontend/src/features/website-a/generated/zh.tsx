// Generated from approved A, SHA-256 5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33.
// Run: node scripts/website-a/generate.mjs <baseline.html>
import { lazy, useEffect } from "react";
import { useWebsite } from "../website-context";
import WebsiteHeader from "./zh/WebsiteHeader";
import WebsiteFooter from "./zh/WebsiteFooter";
const HomePage = lazy(() => import("./zh/HomePage"));
const NyxbotPage = lazy(() => import("./zh/NyxbotPage"));
const SolutionsPage = lazy(() => import("./zh/SolutionsPage"));
const HowItWorksPage = lazy(() => import("./zh/HowItWorksPage"));
const SecurityPage = lazy(() => import("./zh/SecurityPage"));
const AgentsPage = lazy(() => import("./zh/AgentsPage"));
const PricingPage = lazy(() => import("./zh/PricingPage"));
const IntegrationsPage = lazy(() => import("./zh/IntegrationsPage"));
const BlogPage = lazy(() => import("./zh/BlogPage"));
const ContactPage = lazy(() => import("./zh/ContactPage"));
const NotfoundPage = lazy(() => import("./zh/NotfoundPage"));
const SolutionsRestaurantManagerPage = lazy(
  () => import("./zh/SolutionsRestaurantManagerPage"),
);
const SolutionsHelpDeskPage = lazy(() => import("./zh/SolutionsHelpDeskPage"));
const SolutionsAppointmentBookerPage = lazy(
  () => import("./zh/SolutionsAppointmentBookerPage"),
);
const SolutionsCallbackQueuePage = lazy(
  () => import("./zh/SolutionsCallbackQueuePage"),
);
const SolutionsStockAnalystPage = lazy(
  () => import("./zh/SolutionsStockAnalystPage"),
);
const SolutionsNewsAggregatorPage = lazy(
  () => import("./zh/SolutionsNewsAggregatorPage"),
);
const SolutionsDeskResearcherPage = lazy(
  () => import("./zh/SolutionsDeskResearcherPage"),
);
const SolutionsXLeadCatcherPage = lazy(
  () => import("./zh/SolutionsXLeadCatcherPage"),
);
const SolutionsBackOfficeClerkPage = lazy(
  () => import("./zh/SolutionsBackOfficeClerkPage"),
);
const BlogWhatIsNyxidPage = lazy(() => import("./zh/BlogWhatIsNyxidPage"));
const BlogLunaBistroPage = lazy(() => import("./zh/BlogLunaBistroPage"));
const BlogNeverSharePasswordsPage = lazy(
  () => import("./zh/BlogNeverSharePasswordsPage"),
);
const BlogFiveJobsPage = lazy(() => import("./zh/BlogFiveJobsPage"));
const BlogScopedKeysPage = lazy(() => import("./zh/BlogScopedKeysPage"));
const BlogXDmsLivePage = lazy(() => import("./zh/BlogXDmsLivePage"));
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
  "/": "AI，超越聊天。",
  "/nyxbot": "直接开口， NyxBot 来办。",
  "/solutions": "你的助手要做什么？",
  "/how-it-works": "NyxID 如何把 AI 安全地连接到你的世界",
  "/security": "你的密码， 永远不会交给 AI。",
  "/agents": "已经在用 AI 智能体？ 把它接入 NyxID。",
  "/pricing": "平台原价， 零加价。",
  "/integrations": "连接你 已经在用的工具。",
  "/blog": "指南、故事与更新",
  "/contact": "联系我们",
  "/404": "页面不存在",
  "/solutions/restaurant-manager": "餐厅经理",
  "/solutions/help-desk": "24 小时客服",
  "/solutions/appointment-booker": "预约助手",
  "/solutions/callback-queue": "回访队列",
  "/solutions/stock-analyst": "股票分析师",
  "/solutions/news-aggregator": "新闻聚合",
  "/solutions/desk-researcher": "案头研究员",
  "/solutions/x-lead-catcher": "X 线索捕手",
  "/solutions/back-office-clerk": "后台文员",
  "/blog/what-is-nyxid": "什么是“智能体网关”？写给企业主的通俗指南",
  "/blog/luna-bistro": "一家 40 座小酒馆如何不再错过深夜订位",
  "/blog/never-share-passwords": "为什么你的 AI 永远不该看到你的密码",
  "/blog/five-jobs": "本月就可以交给 AI 的五项重复工作",
  "/blog/scoped-keys": "限权密钥：只给智能体它需要的权限",
  "/blog/x-dms-live": "产品更新：X（推特）私信已上线",
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
