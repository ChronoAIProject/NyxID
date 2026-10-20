import { useId } from "react";
import { useWebsite } from "./website-context";
export function SolutionActivation() {
  const { language } = useWebsite();
  const id = useId();
  return <span className="solution-activation"><button className="btn lg" type="button" disabled aria-describedby={id}>{language === "zh" ? "使用此方案" : "Use this solution"}</button><span id={id} className="small">{language === "zh" ? "暂未开放" : "Not available yet"}</span></span>;
}
