import type { ReactNode } from "react";
import { GroupedDetail } from "./grouped-detail";
import { sectionNavigation } from "@/lib/section-navigation";
import { translateUi } from "@/i18n/use-ui-translation";

/** Retain each visited editor beneath its URL-addressed ancestors. Scope/bar belong to the caller. */
export function SectionPages<S extends string>({
  sections,
  titles,
  selected,
  root,
  rootTitle,
  navigate,
  render,
  available = {},
  showRootBack = true,
}: {
  readonly sections: readonly S[];
  readonly titles: Record<S, string>;
  readonly selected?: S;
  readonly root?: S;
  readonly rootTitle: string;
  readonly navigate: (section?: S) => void;
  readonly render: (section: S) => ReactNode;
  readonly showRootBack?: boolean;
  readonly available?: Partial<Record<S, boolean>>;
}) {
  const paths = sectionNavigation(sections);
  const ancestry = paths.ancestry(selected);
  return sections
    .filter(
      (section) =>
        section !== root &&
        (!root || section.startsWith(`${root}/`)) &&
        available[section] !== false,
    )
    .map((section) => (
      <GroupedDetail
        key={section}
        draftOwner={`section/${section}`}
        open={ancestry.includes(section)}
        compactHeader={selected === section}
        showBack={showRootBack || Boolean(paths.parent(section))}
        level={paths.ancestry(section).length - (root ? 1 : 0)}
        searchPage={section}
        onReveal={() => navigate(section)}
        title={translateUi(titles[section])}
        backLabel={translateUi(
          paths.parent(section) && paths.parent(section) !== root
            ? titles[paths.parent(section)!]
            : rootTitle,
        )}
        onBack={() => navigate(paths.parent(section))}
      >
        {render(section)}
      </GroupedDetail>
    ));
}
