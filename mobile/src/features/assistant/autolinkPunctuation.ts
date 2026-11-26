import { bareAutolinkBoundary } from "../../../../shared/autolinkPunctuation";
export {
  bareAutolinkBoundary,
  splitBareAutolinks,
} from "../../../../shared/autolinkPunctuation";

/** Call only with linkify provenance; authored Markdown URLs are left intact. */
export function bareAutolinkTarget(value: string): string {
  return value.slice(0, bareAutolinkBoundary(value));
}
