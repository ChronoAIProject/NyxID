import { websiteRelease } from "./release-config";
import { SiteLink } from "./site-link";
import { useId } from "react";
import { useWebsite } from "./website-context";
import { solutionAuthPath } from "./auth-links";
export function SolutionActivation({
  targets = websiteRelease.solutions,
}: {
  targets?: Readonly<Record<string, string | undefined>>;
}) {
  const { language, route, siteHref } = useWebsite();
  const id = useId();
  const target = targets[route?.split("/").at(-1) ?? ""];
  if (target)
    return (
      <SiteLink
        className="btn lg"
        data-solution-activation={route.split("/").at(-1)}
        href={siteHref(solutionAuthPath(target))}
      >
        {language === "zh" ? "使用此方案" : "Use this solution"}
      </SiteLink>
    );
  return (
    <span className="solution-activation">
      <button className="btn lg" type="button" disabled aria-describedby={id}>
        {language === "zh" ? "使用此方案" : "Use this solution"}
      </button>
      <span id={id} className="small">
        {language === "zh" ? "暂未开放" : "Not available yet"}
      </span>
    </span>
  );
}
