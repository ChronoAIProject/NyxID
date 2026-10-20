// Generated from approved A, SHA-256 5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33.
// Run: node scripts/website-a/generate.mjs <baseline.html>
import { VisualLink } from "../../visuals/elements";
import { SolutionActivation } from "../../solution-activation";
import { useWebsite } from "../../website-context";
export default function SolutionsCallbackQueuePage() {
  const s = useWebsite();
  void s;
  return (
    <>
      {"\n"}
      <section
        className="hero glowbg"
        style={{
          textAlign: "left",
          paddingBottom: "64px",
          position: "relative",
        }}
      >
        {"\n  "}
        <div className="wrap split">
          {"\n    "}
          <div>
            {"\n      "}
            <div className="crumbs">
              <a href={s.siteHref("/solutions")}>{"方案"}</a>
              <span>{"/"}</span>
              <span>{"客户支持"}</span>
            </div>
            {"\n      "}
            <div className="ic lg g">
              <svg className="i" viewBox="0 0 24 24">
                <path d="M22 16.92v3a2 2 0 0 1-2.18 2 19.79 19.79 0 0 1-8.63-3.07 19.5 19.5 0 0 1-6-6 19.79 19.79 0 0 1-3.07-8.67A2 2 0 0 1 4.11 2h3a2 2 0 0 1 2 1.72c.13.96.36 1.9.7 2.81a2 2 0 0 1-.45 2.11L8.09 9.91a16 16 0 0 0 6 6l1.27-1.27a2 2 0 0 1 2.11-.45c.91.34 1.85.57 2.81.7A2 2 0 0 1 22 16.92z"></path>
              </svg>
            </div>
            {"\n      "}
            <h1
              className="h1"
              style={{ fontSize: "var(--fs-h1)", marginTop: "20px" }}
            >
              {"回访队列"}
            </h1>
            {"\n      "}
            <p className="lead" style={{ margin: "16px 0 0" }}>
              {"按名单逐一回电客户，用 AI 语音完成通话，并记录结果。"}
            </p>
            {"\n      "}
            <p
              style={{ marginTop: "14px", fontWeight: "600", fontSize: "17px" }}
            >
              {"每个回电都打到，每个结果都记下。"}
            </p>
            {"\n      "}
            <div
              style={{
                display: "flex",
                gap: "10px",
                flexWrap: "wrap",
                marginTop: "28px",
              }}
            >
              <SolutionActivation />
              <a className="btn lg" href={s.siteHref("/how-it-works")}>
                {"工作原理"}
              </a>
            </div>
            {"\n      "}
            <div className="pills" style={{ marginTop: "20px" }}>
              <span className="badge b-ro">{"● 电话"}</span>
              <span className="badge b-m">{"Google 表格"}</span>
              <span className="badge b-m">{"Twilio"}</span>
              <span className="badge b-m">{"ElevenLabs"}</span>
              <span className="badge b-m">{"折扣与退款"}</span>
            </div>
            {"\n    "}
          </div>
          {"\n    "}
          <div className="phone">
            <div className="ph-h">
              <div className="av">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M22 16.92v3a2 2 0 0 1-2.18 2 19.79 19.79 0 0 1-8.63-3.07 19.5 19.5 0 0 1-6-6 19.79 19.79 0 0 1-3.07-8.67A2 2 0 0 1 4.11 2h3a2 2 0 0 1 2 1.72c.13.96.36 1.9.7 2.81a2 2 0 0 1-.45 2.11L8.09 9.91a16 16 0 0 0 6 6l1.27-1.27a2 2 0 0 1 2.11-.45c.91.34 1.85.57 2.81.7A2 2 0 0 1 22 16.92z"></path>
                </svg>
              </div>
              <div>
                <b style={{ fontSize: "14px" }}>{"回访队列"}</b>
                <div className="small" style={{ fontSize: "11.5px" }}>
                  {"电话 · 回访队列"}
                </div>
              </div>
            </div>
            <div className="msgs">
              <div className="m sys">{"工作日上午 10:00 · 定时运行"}</div>
              <div className="m tool">{"✓ 14 位客户待回电"}</div>
              <div className="m out">{"正在呼叫 Maria L. ·“询问周末时段”"}</div>
              <div className="m tool">{"✓ 已预约周六下午 2 点"}</div>
              <div className="m out">
                {"James K. 希望报价打折，同意打九折吗？"}
              </div>
              <div className="m in">{"同意"}</div>
              <div className="m tool">{"✓ 已批准 · 正在回电"}</div>
            </div>
          </div>
          {"\n  "}
        </div>
        {"\n"}
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap">
          {"\n  "}
          <div className="over v">{"运行方式"}</div>
          <h2 className="h2" style={{ margin: "10px 0 32px" }}>
            {"从收到消息到完成"}
          </h2>
          {"\n  "}
          <div className="flow">
            <div className="fn ">
              <div className="k">{"触发"}</div>
              <b>{"“待回电”表新增行"}</b>
              {"或每个工作日上午 10 点"}
            </div>
            <div className="fn ">
              <div className="k">{"助手"}</div>
              <b>{"读取每条备注"}</b>
              {"准备通话内容"}
            </div>
            <div className="fn ">
              <div className="k">{"执行"}</div>
              <b>{"拨打电话"}</b>
              {"Twilio + AI 语音"}
            </div>
            <div className="fn gate">
              <div className="k">{"需你确认"}</div>
              <b>{"折扣或退款"}</b>
              {"手机上点“是”或“否”"}
            </div>
            <div className="fn ">
              <div className="k">{"结果"}</div>
              <b>{"记录结果"}</b>
              {"已预约 / 未接 / 稍后再打"}
            </div>
          </div>
          {"\n"}
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap split">
          {"\n  "}
          <div>
            <div className="over v">{"默认安全"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"它被允许做什么"}
            </h2>
            <p className="lead" style={{ marginTop: "14px" }}>
              {"这些权限已经预设好。上线前可以修改，之后也能一键调整。"}
            </p>
            {"\n  "}
            <div
              style={{
                display: "flex",
                gap: "8px",
                flexWrap: "wrap",
                marginTop: "22px",
              }}
            >
              <span className="badge b-ok">{"允许"}</span>
              <span className="badge b-ro">{"只读"}</span>
              <span className="badge b-ask">{"先问我"}</span>
              <span className="badge b-no">{"禁止"}</span>
            </div>
            {"\n  "}
            <a
              className="more"
              style={{ marginTop: "22px" }}
              href={s.siteHref("/security")}
            >
              {"了解权限如何运作 "}
              <svg className="i" viewBox="0 0 24 24">
                <path d="M5 12h14M12 5l7 7-7 7"></path>
              </svg>
            </a>
          </div>
          {"\n  "}
          <div className="perm-card">
            <div className="pc-h">
              <div>
                <b>{"回访队列"}</b>
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
              <span className="gl" style={{ background: "#0F9D58" }}>
                {"SH"}
              </span>
              <div>
                {"Google 表格"}
                <small>{"读取通话名单"}</small>
              </div>
              <span className="badge b-ro">{"只读"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#0F9D58" }}>
                {"SH"}
              </span>
              <div>
                {"Google 表格"}
                <small>{"写入结果"}</small>
              </div>
              <span className="badge b-ok">{"允许"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#F22F46" }}>
                {"TW"}
              </span>
              <div>
                {"Twilio"}
                <small>{"拨打名单上的号码"}</small>
              </div>
              <span className="badge b-ok">{"允许"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#18181B" }}>
                {"11"}
              </span>
              <div>
                {"ElevenLabs"}
                <small>{"自然语音"}</small>
              </div>
              <span className="badge b-ok">{"允许"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#52525B" }}>
                {"$"}
              </span>
              <div>
                {"折扣与退款"}
                <small>{"提供退款或优惠"}</small>
              </div>
              <span className="badge b-ask">{"先问我"}</span>
            </div>
            <div className="small" style={{ marginTop: "12px" }}>
              {"底层原理：带有操作范围和审批模式的 NyxID Agent Key。"}
            </div>
          </div>
          {"\n"}
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap split">
          {"\n  "}
          <div>
            <div className="over v">{"设置"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"你只需填写几项信息"}
            </h2>
            <p className="lead" style={{ marginTop: "14px" }}>
              {"用大白话回答即可。之后在聊天中告诉 NyxBot 就能随时修改。"}
            </p>
          </div>
          {"\n  "}
          <div className="card">
            <ul className="checks">
              <li>
                <span className="ck">{"1"}</span>
                <div>
                  <b>{"你的回电表格"}</b>
                </div>
              </li>
              <li>
                <span className="ck">{"2"}</span>
                <div>
                  <b>{"拨打时间"}</b>
                </div>
              </li>
              <li>
                <span className="ck">{"3"}</span>
                <div>
                  <b>{"助手可以提供的优惠"}</b>
                </div>
              </li>
              <li>
                <span className="ck">{"4"}</span>
                <div>
                  <b>{"简短话术或语气指南"}</b>
                </div>
              </li>
            </ul>
          </div>
          {"\n"}
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap">
          <div className="over v">{"价格"}</div>
          <h2 className="h2" style={{ margin: "10px 0 28px" }}>
            {"费用说明"}
          </h2>
          {"\n  "}
          <div className="grid g2">
            {"\n    "}
            <div className="card">
              <span className="badge b-ok">{"● 用量"}</span>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"平台原价"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {
                  "此方案按各服务的收费计费：Google 表格、Twilio、ElevenLabs、折扣与退款以及 AI 模型，零加价。早期体验期间网络费用由我们承担。"
                }
              </p>
              <a
                className="more"
                style={{ marginTop: "12px" }}
                href={s.siteHref("/pricing")}
              >
                {"价格说明 "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </a>
            </div>
            {"\n    "}
            <div className="card">
              <span className="badge b-m">{"企业"}</span>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"定制套餐"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {
                  "规模化运行回访队列？我们会根据你的用量、部署和支持需求定制套餐。"
                }
              </p>
              <a
                className="more"
                style={{ marginTop: "12px" }}
                href={s.siteHref("/contact?topic=bundle")}
              >
                {"联系我们 "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </a>
            </div>
            {"\n  "}
          </div>
          {"\n"}
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap" style={{ maxWidth: "860px" }}>
          <div className="over v">{"常见问题"}</div>
          <h2 className="h2" style={{ margin: "10px 0 18px" }}>
            {"关于此方案"}
          </h2>
          <details>
            <summary>{"是通过 NyxID 直接拨打的吗？"}</summary>
            <p>
              {
                "通话通过你连接的 Twilio 和 ElevenLabs 账号进行；NyxID 保管密钥，并限制可以拨打的号码。"
              }
            </p>
          </details>
          <details>
            <summary>{"它能拨打任何号码吗？"}</summary>
            <p>{"不能。此方案中只能拨打名单上的号码。"}</p>
          </details>
          <details>
            <summary>{"会录音吗？"}</summary>
            <p>{"只有你在 Twilio 中开启录音才会。"}</p>
          </details>
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap">
          <div className="sechead">
            <h2 className="h2">{"相关方案"}</h2>
            <a className="btn" href={s.siteHref("/solutions")}>
              {"全部方案 "}
              <svg className="i" viewBox="0 0 24 24">
                <path d="M5 12h14M12 5l7 7-7 7"></path>
              </svg>
            </a>
          </div>
          <div className="grid g3">
            <VisualLink
              className="card sol "
              href={s.siteHref("/solutions/help-desk")}
              effect="card"
            >
              {"\n  "}
              <div className="top">
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M3 14h3a2 2 0 0 1 2 2v3a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-7a9 9 0 0 1 18 0v7a2 2 0 0 1-2 2h-1a2 2 0 0 1-2-2v-3a2 2 0 0 1 2-2h3"></path>
                  </svg>
                </div>
                <span className="badge b-m">{"客户支持"}</span>
              </div>
              {"\n  "}
              <h3>{"24 小时客服"}</h3>
              <p>
                {"随时回答订单、配送和政策问题，把棘手的问题转给你的团队。"}
              </p>
              {"\n  "}
              <div className="tags">
                <span className="badge b-ok">{"● Telegram"}</span>
                <span className="badge b-ok">{"● X"}</span>
              </div>
              {"\n  "}
              <div className="small" style={{ fontSize: "12px" }}>
                {"平台原价 · 零加价"}
              </div>
              {"\n  "}
              <div className="foot">
                <span>
                  <svg className="i" viewBox="0 0 24 24">
                    <rect x="3" y="11" width="18" height="11" rx="2"></rect>
                    <path d="M7 11V7a5 5 0 0 1 10 0v4"></path>
                  </svg>
                  {"禁止：办理退款"}
                </span>
                <span className="more">
                  {"查看 "}
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M5 12h14M12 5l7 7-7 7"></path>
                  </svg>
                </span>
              </div>
            </VisualLink>
          </div>
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap">
          <div className="band">
            <h2 className="h2">{"试用回访队列"}</h2>
            <p style={{ opacity: ".88", marginTop: "12px", fontSize: "17px" }}>
              {"几分钟完成设置，随时可以关闭。"}
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
