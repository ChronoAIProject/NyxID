// Generated from approved A, SHA-256 5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33.
// Run: node scripts/website-a/generate.mjs <baseline.html>
import { SiteLink } from "../../site-link";
import { useWebsite } from "../../website-context";
export default function NotfoundPage() {
  const s = useWebsite();
  void s;
  return (
    <>
      <section className="s" style={{ textAlign: "center", minHeight: "60vh" }}>
        <div className="wrap">
          <h1 className="h2">{"页面不存在"}</h1>
          <SiteLink
            className="btn"
            style={{ marginTop: "20px" }}
            href={s.siteHref("/")}
          >
            {"返回首页"}
          </SiteLink>
        </div>
      </section>
    </>
  );
}
