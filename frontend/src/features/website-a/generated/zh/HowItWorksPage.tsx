// Generated from approved A, SHA-256 5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33.
// Run: node scripts/website-a/generate.mjs <baseline.html>
import { useWebsite } from "../../website-context";
export default function HowItWorksPage() {
  const s = useWebsite();
  void s;
  return (
    <>
      {"\n"}
      <section className="hero glowbg" style={{ position: "relative" }}>
        {"\n  "}
        <div className="wrap">
          <div className="over v">{"工作原理"}</div>
          <h1
            className="h1"
            style={{ marginTop: "14px", fontSize: "var(--fs-h1)" }}
          >
            {"NyxID 如何把 AI"}
            <br />
            <span className="grad">{"安全地"}</span>
            {"连接到你的世界"}
          </h1>
          <p className="lead">
            {
              "四个层次，一条原则：AI 只能得到你允许的东西。下面是从“接下这个订位”到“完成”之间发生的一切。"
            }
          </p>
        </div>
        {"\n"}
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap">
          {"\n  "}
          <div
            className="center"
            style={{ textAlign: "center", maxWidth: "720px" }}
          >
            <div className="over v">{"全景图"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"四个层次"}
            </h2>
          </div>
          {"\n  "}
          <div
            className="grid g4"
            style={{ marginTop: "40px", position: "relative" }}
          >
            {"\n    "}
            <div className="card" style={{}}>
              <div className="ic ">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"></path>
                </svg>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"1 · 在哪里提出"}
              </h3>
              <p className="sub" style={{ fontSize: "14px" }}>
                {
                  "Telegram、X 私信、NyxID 应用，或通过 API / MCP 从你自己的应用发起。"
                }
              </p>
              <div className="pills" style={{ marginTop: "14px", gap: "6px" }}>
                <span className="badge b-m">{"Telegram"}</span>
                <span className="badge b-m">{"X"}</span>
                <span className="badge b-m">{"应用"}</span>
                <span className="badge b-m">{"API · MCP"}</span>
              </div>
            </div>
            <div className="card" style={{}}>
              <div className="ic ">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M12 3l1.9 5.8L20 11l-6.1 2.2L12 19l-1.9-5.8L4 11l6.1-2.2z"></path>
                </svg>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"2 · NyxBot 思考"}
              </h3>
              <p className="sub" style={{ fontSize: "14px" }}>
                {"理解请求，选择合适的专属助手，规划步骤。"}
              </p>
              <div className="pills" style={{ marginTop: "14px", gap: "6px" }}>
                <span className="badge b-m">{"专属助手"}</span>
                <span className="badge b-m">{"记忆"}</span>
                <span className="badge b-m">{"计划任务"}</span>
              </div>
            </div>
            <div
              className="card"
              style={{
                borderColor: "var(--accentb)",
                boxShadow: "0 0 0 3px var(--accent)",
              }}
            >
              <div className="ic g">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"></path>
                </svg>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"3 · NyxID 把关"}
              </h3>
              <p className="sub" style={{ fontSize: "14px" }}>
                {
                  "网关检查限权密钥，必要时征求你的同意，注入隐藏的登录信息并记录调用。"
                }
              </p>
              <div className="pills" style={{ marginTop: "14px", gap: "6px" }}>
                <span className="badge b-m">{"保险库"}</span>
                <span className="badge b-m">{"限权密钥"}</span>
                <span className="badge b-m">{"审批"}</span>
                <span className="badge b-m">{"审计日志"}</span>
              </div>
            </div>
            <div className="card" style={{}}>
              <div className="ic ">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M12 22v-5M9 8V2M15 8V2M18 8v5a6 6 0 0 1-12 0V8z"></path>
                </svg>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"4 · 完成工作"}
              </h3>
              <p className="sub" style={{ fontSize: "14px" }}>
                {
                  "调用服务 API、AI 模型，或在你的电脑或托管虚拟机上使用浏览器。"
                }
              </p>
              <div className="pills" style={{ marginTop: "14px", gap: "6px" }}>
                <span className="badge b-m">{"100+ API"}</span>
                <span className="badge b-m">{"AI 模型"}</span>
                <span className="badge b-m">{"电脑"}</span>
              </div>
            </div>
            {"\n  "}
          </div>
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap split">
          {"\n  "}
          <div>
            <div className="over v">{"一次请求的全过程"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"大约一秒内发生的事"}
            </h2>
            <p className="lead" style={{ marginTop: "14px" }}>
              {
                "每一个操作，无论是订桌还是点击网页上的按钮，都要经过同样的检查。"
              }
            </p>
          </div>
          {"\n  "}
          <div
            className="card log"
            style={{ fontFamily: "var(--sans)", fontSize: "14px" }}
          >
            <div className="r">
              <span className="tm mono">{"01"}</span>
              <span>{"NyxBot 请求创建日历事件"}</span>
              <span className="badge b-m">{"请求"}</span>
            </div>
            <div className="r">
              <span className="tm mono">{"02"}</span>
              <span>{"这把密钥能创建事件吗？"}</span>
              <span className="badge b-ok">{"是"}</span>
            </div>
            <div className="r">
              <span className="tm mono">{"03"}</span>
              <span>{"这个操作需要你确认吗？"}</span>
              <span className="badge b-ro">{"否"}</span>
            </div>
            <div className="r">
              <span className="tm mono">{"04"}</span>
              <span>{"注入隐藏的 Google 登录凭据"}</span>
              <span className="badge b-v">{"保险库"}</span>
            </div>
            <div className="r">
              <span className="tm mono">{"05"}</span>
              <span>{"发送到 Google 日历"}</span>
              <span className="badge b-ok">{"200 OK"}</span>
            </div>
            <div className="r">
              <span className="tm mono">{"06"}</span>
              <span>{"写入防篡改日志"}</span>
              <span className="badge b-ok">{"已记录"}</span>
            </div>
          </div>
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap split">
          {"\n  "}
          <div>
            <div className="stepn">{"1"}</div>
            <h2
              className="h3"
              style={{ margin: "16px 0 10px", fontSize: "var(--fs-h2-sub)" }}
            >
              {"告诉 NyxBot 你想要什么"}
            </h2>
            <p className="lead">
              {
                "用大白话，在 NyxID 应用或 Telegram 里说。NyxBot 会判断需要哪些服务，以及哪些东西绝不能碰。"
              }
            </p>
            <a
              className="more"
              style={{ marginTop: "18px" }}
              href={s.siteHref("/solutions")}
            >
              {"或从现成方案开始 "}
              <svg className="i" viewBox="0 0 24 24">
                <path d="M5 12h14M12 5l7 7-7 7"></path>
              </svg>
            </a>
          </div>
          {"\n  "}
          <div className="phone">
            <div className="ph-h">
              <div className="av">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M12 3l1.9 5.8L20 11l-6.1 2.2L12 19l-1.9-5.8L4 11l6.1-2.2z"></path>
                </svg>
              </div>
              <div>
                <b style={{ fontSize: "14px" }}>{"NyxBot"}</b>
                <div className="small" style={{ fontSize: "11.5px" }}>
                  {"你的助手"}
                </div>
              </div>
            </div>
            <div className="msgs">
              <div className="m in">
                {
                  "在 Telegram 上回答客户问题，在我的表格里查订单。绝对不要退款。"
                }
              </div>
              <div className="m out">
                {
                  "明白。我会创建一个客服助手：允许 Telegram 回复，订单表只读，禁止退款。请在这里连接你的表格："
                }
              </div>
              <div className="m tool">{"🔗 安全登录链接 · Google 表格"}</div>
            </div>
          </div>
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap split rev">
          {"\n  "}
          <div>
            <div className="stepn">{"2"}</div>
            <h2
              className="h3"
              style={{ margin: "16px 0 10px", fontSize: "var(--fs-h2-sub)" }}
            >
              {"安全连接你的工具"}
            </h2>
            <p className="lead">
              {
                "你在 Google、Telegram 或 X 自己的页面上登录，就像平常点“使用 Google 登录”一样。NyxID 把登录信息锁在加密保险库里，AI 永远看不到。对于没有 API 的网站，可以连接一台电脑或托管虚拟机。"
              }
            </p>
            {"\n  "}
            <ul className="checks" style={{ marginTop: "22px" }}>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <rect x="3" y="11" width="18" height="11" rx="2"></rect>
                    <path d="M7 11V7a5 5 0 0 1 10 0v4"></path>
                  </svg>
                </span>
                <div>
                  <b>{"加密保险库"}</b>
                  <span>{"AES-256 加密，支持云端密钥管理。"}</span>
                </div>
              </li>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <rect x="2" y="3" width="20" height="14" rx="2"></rect>
                    <path d="M8 21h8M12 17v4"></path>
                  </svg>
                </span>
                <div>
                  <b>{"电脑或托管虚拟机"}</b>
                  <span>{"用于没有 API 的后台和应用。"}</span>
                </div>
              </li>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M18.36 6.64a9 9 0 1 1-12.73 0"></path>
                    <path d="M12 2v10"></path>
                  </svg>
                </span>
                <div>
                  <b>{"随时断开"}</b>
                  <span>{"一键即可在所有地方移除连接。"}</span>
                </div>
              </li>
            </ul>
          </div>
          {"\n  "}
          <div
            className="card mock"
            style={{
              maxWidth: "420px",
              width: "100%",
              margin: "0 auto",
              padding: "28px",
              textAlign: "center",
            }}
          >
            {"\n    "}
            <div
              style={{
                display: "flex",
                justifyContent: "center",
                alignItems: "center",
                gap: "14px",
              }}
            >
              <span className="gl" style={{ background: "#4285F4" }}>
                {"31"}
              </span>
              <span className="small">{"⟷"}</span>
              <img
                src="/website-a/fa92f171d1d90a9d.svg"
                width="30"
                onError={s.imageFallback}
              />
            </div>
            {"\n    "}
            <h3 style={{ fontSize: "var(--fs-h3)", margin: "18px 0 6px" }}>
              {"连接 Google 日历"}
            </h3>
            <p className="small">{"Luna Bistro 的助手将能够："}</p>
            {"\n    "}
            <div
              style={{ textAlign: "left", margin: "16px 0" }}
              className="grid"
            >
              <div
                style={{
                  display: "flex",
                  justifyContent: "space-between",
                  fontSize: "14px",
                  padding: "6px 0",
                  borderBottom: "1px solid var(--border)",
                }}
              >
                {"查看空闲时间"}
                <span className="badge b-ok">{"允许"}</span>
              </div>
              <div
                style={{
                  display: "flex",
                  justifyContent: "space-between",
                  fontSize: "14px",
                  padding: "6px 0",
                  borderBottom: "1px solid var(--border)",
                }}
              >
                {"创建预约"}
                <span className="badge b-ok">{"允许"}</span>
              </div>
              <div
                style={{
                  display: "flex",
                  justifyContent: "space-between",
                  fontSize: "14px",
                  padding: "6px 0",
                  borderBottom: "1px solid var(--border)",
                }}
              >
                {"取消预约"}
                <span className="badge b-ask">{"先问我"}</span>
              </div>
            </div>
            {"\n    "}
            <button className="btn p" style={{ width: "100%" }} type="button">
              {"使用 Google 登录"}
            </button>
            <p className="small" style={{ marginTop: "10px" }}>
              {"安全链接 · 15 分钟后失效"}
            </p>
            {"\n  "}
          </div>
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap split">
          {"\n  "}
          <div>
            <div className="stepn">{"3"}</div>
            <h2
              className="h3"
              style={{ margin: "16px 0 10px", fontSize: "var(--fs-h2-sub)" }}
            >
              {"设定规则"}
            </h2>
            <p className="lead">
              {
                "每个操作都有四种设置之一。方案自带安全的默认值，大多数店主无需改动。"
              }
            </p>
            {"\n  "}
            <div className="grid g2" style={{ marginTop: "24px" }}>
              <div className="card" style={{ padding: "14px" }}>
                <span className="badge b-ok">{"允许"}</span>
                <p className="small" style={{ marginTop: "8px" }}>
                  {"自动执行"}
                </p>
              </div>
              <div className="card" style={{ padding: "14px" }}>
                <span className="badge b-ro">{"只读"}</span>
                <p className="small" style={{ marginTop: "8px" }}>
                  {"可以看，不能改"}
                </p>
              </div>
              <div className="card" style={{ padding: "14px" }}>
                <span className="badge b-ask">{"先问我"}</span>
                <p className="small" style={{ marginTop: "8px" }}>
                  {"先发给你确认"}
                </p>
              </div>
              <div className="card" style={{ padding: "14px" }}>
                <span className="badge b-no">{"禁止"}</span>
                <p className="small" style={{ marginTop: "8px" }}>
                  {"永远不能做"}
                </p>
              </div>
            </div>
          </div>
          {"\n  "}
          <div className="perm-card">
            <div className="pc-h">
              <div>
                <b>{"餐厅经理"}</b>
                <div className="small">{"这个助手能做什么"}</div>
              </div>
              <span className="badge b-v">
                <svg className="i" viewBox="0 0 24 24">
                  <circle cx="7.5" cy="15.5" r="5.5"></circle>
                  <path d="m21 2-9.6 9.6M15.5 7.5l3 3L22 7l-3-3"></path>
                </svg>
                {" 限权密钥"}
              </span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#229ED9" }}>
                {"TG"}
              </span>
              <div>
                {"Telegram"}
                <small>{"回复客人"}</small>
              </div>
              <span className="badge b-ok">{"允许"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#4285F4" }}>
                {"31"}
              </span>
              <div>
                {"Google 日历"}
                <small>{"查空位并订桌"}</small>
              </div>
              <span className="badge b-ok">{"允许"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#0F9D58" }}>
                {"SH"}
              </span>
              <div>
                {"Google 表格"}
                <small>{"客人备注与过敏信息"}</small>
              </div>
              <span className="badge b-ok">{"允许"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#4285F4" }}>
                {"31"}
              </span>
              <div>
                {"Google 日历"}
                <small>{"取消订位"}</small>
              </div>
              <span className="badge b-ask">{"先问我"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#52525B" }}>
                {"$"}
              </span>
              <div>
                {"支付"}
                <small>{"收取订金"}</small>
              </div>
              <span className="badge b-no">{"禁止"}</span>
            </div>
          </div>
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap split rev">
          {"\n  "}
          <div>
            <div className="stepn">{"4"}</div>
            <h2
              className="h3"
              style={{ margin: "16px 0 10px", fontSize: "var(--fs-h2-sub)" }}
            >
              {"在客户所在的地方上线"}
            </h2>
            <p className="lead">
              {
                "在 Telegram 内点几下即可创建机器人，或连接你的 X 账号。把链接放到网站、Instagram 简介或小票上。"
              }
            </p>
            {"\n  "}
            <div className="pills" style={{ marginTop: "20px" }}>
              <span className="badge b-ok">{"● Telegram 已上线"}</span>
              <span className="badge b-ok">{"● X 私信已上线"}</span>
              <span className="badge b-m">
                {"WhatsApp、Discord、Slack 即将推出"}
              </span>
            </div>
          </div>
          {"\n  "}
          <div className="phone">
            <div className="ph-h">
              <div className="av">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M3 2v7c0 1.1.9 2 2 2h4a2 2 0 0 0 2-2V2"></path>
                  <path d="M7 2v20"></path>
                  <path d="M21 15V2a5 5 0 0 0-5 5v6c0 1.1.9 2 2 2h3Zm0 0v7"></path>
                </svg>
              </div>
              <div>
                <b style={{ fontSize: "14px" }}>{"Luna Bistro"}</b>
                <div className="small" style={{ fontSize: "11.5px" }}>
                  {"Telegram · @LunaBistroBot"}
                </div>
              </div>
            </div>
            <div className="msgs">
              <div className="m sys">{"@LunaBistroBot · 已连接 ✓"}</div>
              <div className="m in">{"周日营业吗？"}</div>
              <div className="m out">
                {"营业！周日中午 12 点到晚上 10 点。要帮你订桌吗？"}
              </div>
            </div>
          </div>
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap split">
          {"\n  "}
          <div>
            <div className="stepn">{"5"}</div>
            <h2
              className="h3"
              style={{ margin: "16px 0 10px", fontSize: "var(--fs-h2-sub)" }}
            >
              {"用手机掌控一切"}
            </h2>
            <p className="lead">
              {
                "高风险操作有审批卡片，每天有工作汇总，随时可以查看完整的操作记录。"
              }
            </p>
            {"\n  "}
            <ul className="checks" style={{ marginTop: "22px" }}>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"></path>
                    <path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"></path>
                  </svg>
                </span>
                <div>
                  <b>{"审批卡片"}</b>
                  <span>{"在 NyxID 应用或 Telegram 中点“同意”或“拒绝”。"}</span>
                </div>
              </li>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M8 6h13M8 12h13M8 18h13M3 6h.01M3 12h.01M3 18h.01"></path>
                  </svg>
                </span>
                <div>
                  <b>{"操作记录"}</b>
                  <span>{"每个操作、谁做的、何时做的。"}</span>
                </div>
              </li>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M18.36 6.64a9 9 0 1 1-12.73 0"></path>
                    <path d="M12 2v10"></path>
                  </svg>
                </span>
                <div>
                  <b>{"随时暂停"}</b>
                  <span>{"一键停用助手，之后可随时重新启用。"}</span>
                </div>
              </li>
            </ul>
          </div>
          {"\n  "}
          <div
            className="card mock"
            style={{
              maxWidth: "380px",
              width: "100%",
              margin: "0 auto",
              padding: "24px",
            }}
          >
            {"\n    "}
            <div style={{ display: "flex", alignItems: "center", gap: "10px" }}>
              <div className="ic">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"></path>
                  <path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"></path>
                </svg>
              </div>
              <div>
                <b>{"需要审批"}</b>
                <div className="small">{"餐厅经理 · 刚刚"}</div>
              </div>
            </div>
            {"\n    "}
            <p style={{ margin: "16px 0", fontSize: "15px" }}>
              {"Sarah 想"}
              <b>{"取消"}</b>
              {"周日晚上 7:30 的 4 人桌。确认取消吗？"}
            </p>
            {"\n    "}
            <div className="grid g2">
              <button
                className="btn"
                style={{ color: "var(--err)" }}
                type="button"
              >
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M18 6 6 18M6 6l12 12"></path>
                </svg>
                {" 拒绝"}
              </button>
              <button
                className="btn"
                style={{ color: "var(--ok)" }}
                type="button"
              >
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M20 6 9 17l-5-5"></path>
                </svg>
                {" 同意"}
              </button>
            </div>
            {"\n    "}
            <p
              className="small"
              style={{ marginTop: "12px", textAlign: "center" }}
            >
              {"验证码 4821 · 10 分钟后失效"}
            </p>
            {"\n  "}
          </div>
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap">
          {"\n  "}
          <div className="center" style={{ textAlign: "center" }}>
            <div className="over v">{"使用 NyxID 的三种方式"}</div>
            <h2
              className="h2"
              style={{ marginTop: "10px", marginBottom: "36px" }}
            >
              {"选择适合你的方式"}
            </h2>
          </div>
          {"\n  "}
          <div className="grid g3">
            {"\n    "}
            <a className="card eq" href={s.siteHref("/nyxbot")}>
              <span className="badge b-v">{"无需代码"}</span>
              <h3
                className="h3"
                style={{ fontSize: "var(--fs-h3)", margin: "14px 0 8px" }}
              >
                {"交给 NyxBot"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {"描述工作或选择方案，连接 Telegram，搞定。适合店主和团队。"}
              </p>
              <span className="more" style={{ marginTop: "16px" }}>
                {"认识 NyxBot "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </span>
            </a>
            {"\n    "}
            <a className="card eq" href={s.siteHref("/agents")}>
              <span className="badge b-ro">{"低代码"}</span>
              <h3
                className="h3"
                style={{ fontSize: "var(--fs-h3)", margin: "14px 0 8px" }}
              >
                {"你的 AI 工具 + NyxID"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {
                  "通过 MCP 把 Claude、ChatGPT 或 Cursor 接入 NyxID，安全使用你的服务。"
                }
              </p>
              <span className="more" style={{ marginTop: "16px" }}>
                {"MCP 设置 "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </span>
            </a>
            {"\n    "}
            <a className="card eq" href={s.siteHref("/agents")}>
              <span className="badge b-m">{"你自己的代码"}</span>
              <h3
                className="h3"
                style={{ fontSize: "var(--fs-h3)", margin: "14px 0 8px" }}
              >
                {"从你的应用调用"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {
                  "你的脚本或应用用一把限权密钥，通过 NyxID 访问 100 多种服务和 AI 模型。"
                }
              </p>
              <span className="more" style={{ marginTop: "16px" }}>
                {"了解方法 "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </span>
            </a>
            {"\n  "}
          </div>
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap" style={{ maxWidth: "860px" }}>
          <div className="over v">{"常见问题"}</div>
          <h2 className="h2" style={{ margin: "10px 0 18px" }}>
            {"常见问题"}
          </h2>
          <details>
            <summary>{"需要写代码吗？"}</summary>
            <p>
              {
                "不需要。告诉 NyxBot 你的需求，或用表单和自然语言说明设置方案即可。"
              }
            </p>
          </details>
          <details>
            <summary>{"设置需要多久？"}</summary>
            <p>{"账号准备好后，大多数方案只需几分钟。"}</p>
          </details>
          <details>
            <summary>{"如果我用的网站没有 API 怎么办？"}</summary>
            <p>
              {
                "连接一台电脑或托管虚拟机，NyxBot 就能在浏览器中使用该网站，你保存的登录信息始终隐藏。"
              }
            </p>
          </details>
          <details>
            <summary>{"用的是哪个 AI 模型？"}</summary>
            <p>{"NyxID 支持主流模型，每个助手都可以单独选择。"}</p>
          </details>
          <details>
            <summary>{"如果它出错了怎么办？"}</summary>
            <p>
              {"高风险操作需要你确认，一切都有记录，你也可以立即暂停助手。"}
            </p>
          </details>
          <details>
            <summary>{"我的团队也能用吗？"}</summary>
            <p>{"可以。通过组织功能，你可以按角色权限共享助手和连接。"}</p>
          </details>
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap">
          <div className="band">
            <h2 className="h2">{"今晚就把重复工作交出去。"}</h2>
            <p style={{ opacity: ".88", marginTop: "12px", fontSize: "17px" }}>
              {"让 NyxBot 设置你的第一项工作，连接 Telegram，即可上线。"}
            </p>
            <div
              style={{
                display: "flex",
                gap: "12px",
                justifyContent: "center",
                flexWrap: "wrap",
                marginTop: "28px",
                position: "relative",
                zIndex: "1",
              }}
            >
              <a className="btn lg" href={s.authHref("register")}>
                {"开始使用"}
              </a>
              <a className="btn lg gh" href={s.siteHref("/solutions")}>
                {"浏览方案"}
              </a>
            </div>
          </div>
        </div>
      </section>
    </>
  );
}
