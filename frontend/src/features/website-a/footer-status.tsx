import { websiteRelease } from "./release-config";
import { useWebsite } from "./website-context";
export function FooterStatus({
  url = websiteRelease.footerStatus,
}: {
  url?: string | null;
}) {
  const { language } = useWebsite();
  return url ? (
    <a
      data-website-feature="footer-status"
      href={url}
      target="_blank"
      rel="noopener noreferrer"
    >
      {language === "zh" ? "服务状态" : "Status"}
    </a>
  ) : null;
}
