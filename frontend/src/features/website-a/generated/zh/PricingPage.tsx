// Generated from approved A, SHA-256 5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33.
// Run: node scripts/website-a/generate.mjs <baseline.html>
import { useWebsite } from "../../website-context";
export default function PricingPage() {
  const s = useWebsite();
  void s;
  return (
    <>
      {"\n"}
      <section className="hero glowbg" style={{ position: "relative" }}>
        {"\n  "}
        <div className="wrap">
          <div className="over v">{"价格"}</div>
          <h1
            className="h1"
            style={{ marginTop: "14px", fontSize: "var(--fs-h1)" }}
          >
            {"平台原价，"}
            <br />
            <span className="grad">{"零加价。"}</span>
          </h1>
          <p className="lead">
            {
              "每一次模型和 API 调用都按服务商公布的价格计费，NyxID 不额外加收任何费用。"
            }
          </p>
          <div className="cta">
            <a className="btn p lg" href={s.authHref("register")}>
              {"开始使用"}
            </a>
            <a className="btn lg" href={s.siteHref("/contact?topic=bundle")}>
              {"联系我们"}
            </a>
          </div>
        </div>
        {"\n"}
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap split">
          {"\n  "}
          <div>
            <div className="over v">{"我们的承诺"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"平台费用，"}
              <span className="grad">{"零加价。"}</span>
            </h2>
            <p className="lead" style={{ marginTop: "14px" }}>
              {
                "当你的智能体通过 NyxID 调用模型或 API 时，你只需支付该服务商的收费。我们绝不在平台费用之上叠加自己的定价。"
              }
            </p>
            {"\n  "}
            <ul className="checks" style={{ marginTop: "24px" }}>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M20 6 9 17l-5-5"></path>
                  </svg>
                </span>
                <div>
                  <b>{"原价透传"}</b>
                  <span>{"按服务商公布的价格原样计费。"}</span>
                </div>
              </li>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M2 12s3-7 10-7 10 7 10 7-3 7-10 7-10-7-10-7Z"></path>
                    <circle cx="12" cy="12" r="3"></circle>
                  </svg>
                </span>
                <div>
                  <b>{"逐项列明"}</b>
                  <span>{"每笔费用按智能体、服务和调用逐项列出。"}</span>
                </div>
              </li>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <circle cx="7.5" cy="15.5" r="5.5"></circle>
                    <path d="m21 2-9.6 9.6M15.5 7.5l3 3L22 7l-3-3"></path>
                  </svg>
                </span>
                <div>
                  <b>{"自带密钥"}</b>
                  <span>{"使用你自己的服务商账号，直接向他们付费。"}</span>
                </div>
              </li>
            </ul>
          </div>
          {"\n  "}
          <div className="card" style={{ padding: "28px" }}>
            {"\n    "}
            <div className="over">{"一次调用的费用"}</div>
            {"\n    "}
            <div style={{ display: "grid", marginTop: "18px" }}>
              {"\n      "}
              <div
                style={{
                  display: "flex",
                  justifyContent: "space-between",
                  alignItems: "center",
                  padding: "12px 0",
                  borderBottom: "1px solid var(--border)",
                }}
              >
                <span>{"服务商收费"}</span>
                <span className="mono" style={{}}>
                  {"$1.00"}
                </span>
              </div>
              <div
                style={{
                  display: "flex",
                  justifyContent: "space-between",
                  alignItems: "center",
                  padding: "12px 0",
                  borderBottom: "1px solid var(--border)",
                }}
              >
                <span>{"NyxID 加价"}</span>
                <span className="mono" style={{}}>
                  {"$0.00"}
                </span>
              </div>
              <div
                style={{
                  display: "flex",
                  justifyContent: "space-between",
                  alignItems: "center",
                  padding: "12px 0",
                  borderBottom: "1px solid var(--border)",
                }}
              >
                <span>{"网络费用（早期体验）"}</span>
                <span className="mono" style={{}}>
                  {"$0.00"}
                </span>
              </div>
              <div
                style={{
                  display: "flex",
                  justifyContent: "space-between",
                  alignItems: "center",
                  padding: "12px 0",
                  borderBottom: "1px solid var(--border)",
                  fontWeight: "700",
                  fontSize: "18px",
                  border: "none",
                }}
              >
                <span>{"你支付"}</span>
                <span className="mono" style={{ color: "var(--ptext)" }}>
                  {"$1.00"}
                </span>
              </div>
              {"\n    "}
            </div>
            {"\n    "}
            <p className="small" style={{ marginTop: "14px" }}>
              {"仅作示意。实际价格以各服务商公布的价格为准。"}
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
          <div
            className="center"
            style={{ textAlign: "center", maxWidth: "720px" }}
          >
            <div className="over v">{"计费方式"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"三部分，没有意外"}
            </h2>
          </div>
          {"\n  "}
          <div className="grid g3" style={{ marginTop: "40px" }}>
            {"\n    "}
            <div className="card">
              <span className="badge b-ok">{"● 用量"}</span>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"平台费用"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {
                  "模型、API、通话和短信均按各服务商的价格计费。预充余额，按用量扣除。"
                }
              </p>
            </div>
            {"\n    "}
            <div className="card">
              <span className="badge b-v">{"早期体验"}</span>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"网络费用：我们承担"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {
                  "运行网关需要计算、带宽和存储成本。在摸清真实成本之前，这部分由我们承担。如果将来收取网络费用，也只用于覆盖成本，我们会尽量在生效前提前公布。"
                }
              </p>
            </div>
            {"\n    "}
            <div className="card">
              <span className="badge b-m">{"企业"}</span>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"定制套餐"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {
                  "可预期的支出、用量、SSO、本地凭据节点和优先支持，按你的业务量身定制。"
                }
              </p>
              <a
                className="more"
                style={{ marginTop: "14px" }}
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
          {"\n  "}
          <p
            className="small center"
            style={{ textAlign: "center", marginTop: "20px" }}
          >
            {
              "永久免费：网关、保险库、限权密钥、审批和操作记录。自托管开源版同样免费。"
            }
          </p>
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap" style={{ maxWidth: "860px" }}>
          <div className="over v">{"常见问题"}</div>
          <h2 className="h2" style={{ margin: "10px 0 18px" }}>
            {"价格问题"}
          </h2>
          <details>
            <summary>{"NyxID 靠什么赚钱？"}</summary>
            <p>
              {
                "通过面向企业的定制套餐：用量、部署方式和支持服务，而不是在你的用量上加价。"
              }
            </p>
          </details>
          <details>
            <summary>{"什么是网络费用？"}</summary>
            <p>
              {
                "即运行网关本身的成本：计算、带宽和存储。早期体验期间由我们承担。若将来收取，将按成本定价，我们会尽量提前公布。"
              }
            </p>
          </details>
          <details>
            <summary>{"积分是什么？"}</summary>
            <p>
              {"一种预付余额。按平台价格产生的用量从中扣除，不会有意外账单。"}
            </p>
          </details>
          <details>
            <summary>{"能用我自己的 API 密钥吗？"}</summary>
            <p>
              {"可以。使用你自己的服务商账号时，该服务的费用直接付给服务商。"}
            </p>
          </details>
          <details>
            <summary>{"有订阅费吗？"}</summary>
            <p>{"没有。只为平台用量付费，或联系我们定制套餐。"}</p>
          </details>
          <details>
            <summary>{"可以免费自托管吗？"}</summary>
            <p>
              {
                "可以。NyxID 基于 Apache-2.0 开源，你可以用 Docker 自行部署，也可以使用托管服务。"
              }
            </p>
          </details>
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap">
          <div className="band">
            <h2 className="h2">{"只付平台原价，仅此而已。"}</h2>
            <p style={{ opacity: ".88", marginTop: "12px", fontSize: "17px" }}>
              {"从按用量付费开始，或联系我们为团队定制套餐。"}
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
