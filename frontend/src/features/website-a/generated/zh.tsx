// Generated from approved A, SHA-256 5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33.
// Run: node scripts/website-a/generate.mjs <baseline.html>
import { lazy, Suspense } from "react";
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
};
export const descriptions: Record<string, string> = {
  "/": "把无聊的事交给 AI，你专注真正重要的事。连接你的应用，决定它能碰什么，然后让它开工。",
  "/nyxbot":
    "使用 NyxID 最简单的方式。在应用或 Telegram 里告诉它你需要什么：它会连接你的服务、设定每个助手能做什么，并把重复工作自动化。它不是通用聊天机器人，而是专门为你的应用干活。",
  "/solutions": "为真实业务准备的现成工作，已配好合适的连接和安全的权限。",
  "/how-it-works":
    "四个层次，一条原则：AI 只能得到你允许的东西。下面是从“接下这个订位”到“完成”之间发生的一切。",
  "/security":
    "登录信息加密保存在 NyxID 中。每个助手只拿到你允许操作的限权密钥，高风险操作等你确认，每次调用都有记录。",
  "/agents":
    "Claude、ChatGPT、Cursor、Codex 或你自己的脚本：把它指向 NyxID，就能获得对你的服务、AI 模型和机器的限权访问，而且永远不用持有密码。NyxID 是开源的：可自行部署，也可使用我们的托管服务。",
  "/pricing":
    "每一次模型和 API 调用都按服务商公布的价格计费，NyxID 不额外加收任何费用。",
  "/integrations":
    "30 个类别、145 种应用和 API。每个都通过安全登录链接连接，再精确决定 AI 能用它做什么。",
  "/blog": "让 AI 安全落地的实用思路。",
  "/contact": "关于套餐、自托管或设置有疑问？选一种最方便的方式。",
  "/404": "NyxID",
  "/solutions/restaurant-manager":
    "在 Telegram 上接受订位，查看你的日历，记录忌口并发送提醒。",
  "/solutions/help-desk":
    "随时回答订单、配送和政策问题，把棘手的问题转给你的团队。",
  "/solutions/appointment-booker":
    "提供可预约时段，约到合适的人员，改期需经你确认。",
  "/solutions/callback-queue":
    "按名单逐一回电客户，用 AI 语音完成通话，并记录结果。",
  "/solutions/stock-analyst":
    "盯住你的自选股，扫描市场讨论，用大白话给你简报。只读，从不交易。",
  "/solutions/news-aggregator":
    "每天早上阅读你关注的信息源，去重后发你一份清爽的简报。",
  "/solutions/desk-researcher":
    "搜索、阅读、比较，然后把完整报告写进你的 Google 文档。",
  "/solutions/x-lead-catcher":
    "在 X 私信中回答产品问题，筛选线索并加入你的表格。",
  "/solutions/back-office-clerk":
    "登录没有 API 的供应商和预订后台，补货、下载发票并归档，在你的电脑或托管虚拟机上完成。",
};
export const loaders = {
  "/": () => import("./zh/HomePage"),
  "/nyxbot": () => import("./zh/NyxbotPage"),
  "/solutions": () => import("./zh/SolutionsPage"),
  "/how-it-works": () => import("./zh/HowItWorksPage"),
  "/security": () => import("./zh/SecurityPage"),
  "/agents": () => import("./zh/AgentsPage"),
  "/pricing": () => import("./zh/PricingPage"),
  "/integrations": () => import("./zh/IntegrationsPage"),
  "/blog": () => import("./zh/BlogPage"),
  "/contact": () => import("./zh/ContactPage"),
  "/404": () => import("./zh/NotfoundPage"),
  "/solutions/restaurant-manager": () =>
    import("./zh/SolutionsRestaurantManagerPage"),
  "/solutions/help-desk": () => import("./zh/SolutionsHelpDeskPage"),
  "/solutions/appointment-booker": () =>
    import("./zh/SolutionsAppointmentBookerPage"),
  "/solutions/callback-queue": () => import("./zh/SolutionsCallbackQueuePage"),
  "/solutions/stock-analyst": () => import("./zh/SolutionsStockAnalystPage"),
  "/solutions/news-aggregator": () =>
    import("./zh/SolutionsNewsAggregatorPage"),
  "/solutions/desk-researcher": () =>
    import("./zh/SolutionsDeskResearcherPage"),
  "/solutions/x-lead-catcher": () => import("./zh/SolutionsXLeadCatcherPage"),
  "/solutions/back-office-clerk": () =>
    import("./zh/SolutionsBackOfficeClerkPage"),
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
