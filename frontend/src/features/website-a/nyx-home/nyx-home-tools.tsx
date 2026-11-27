import { useState } from "react";
import { ArrowUpRight, Plus, Search } from "lucide-react";
import {
  catalogEntries,
  catalogGroups,
  matchesIntegration,
} from "./catalog-data";
import { useWebsite } from "../website-context";
import { homeText } from "./home-copy";

export function ToolCatalog() {
  const s = useWebsite();
  const t = (english: Parameters<typeof homeText>[0]) => homeText(english, s.language);
  const [query, setQuery] = useState("");
  const search = query.trim().toLowerCase();
  const visible = catalogEntries.filter((tool) =>
    matchesIntegration(tool, search),
  );

  return (
    <details className="tool-catalog" data-home-resize="">
      <summary className="link catalog-toggle list-none">
        <span className="when-closed">{t("Show all tools")}</span>
        <span className="when-open">{t("Show fewer tools")}</span>
        <span className="catalog-count mono">{catalogEntries.length}</span>
        <Plus size={17} aria-hidden="true" />
      </summary>
      <div className="catalog-body">
        <div className="catalog-head">
          <p className="mono" role="status">
            {search
              ? s.language === "zh" ? `找到 ${visible.length} 个工具` : `${visible.length} ${visible.length === 1 ? "tool" : "tools"} found`
              : t("Connect with your own accounts or credentials.")}
          </p>
          <label className="catalog-search">
            <Search size={16} aria-hidden="true" />
            <input
              type="search"
              aria-label={t("Search tools")}
              placeholder={t("Find an integration")}
              value={query}
              onChange={(event) => setQuery(event.target.value)}
            />
          </label>
        </div>
        {visible.length === 0 && (
          <p className="catalog-empty">{t("No tools found. Try another name.")}</p>
        )}
        {catalogGroups.map((group) => {
          const tools = group.entries.filter((tool) =>
            matchesIntegration(tool, search),
          );
          if (!tools.length) return null;
          return (
            <section
              className="catalog-group"
              key={group.id}
              aria-label={s.language === "zh" ? group.label_zh : `${group.label} tools`}
            >
              <h3 className="catalog-group-label mono">
                {s.language === "zh" ? group.label_zh : group.label}
                <span>{tools.length}</span>
              </h3>
              <ul className="tool-grid">
                {tools.map((tool) => (
                  <li className="tile catalog-tile" key={tool.id}>
                    {tool.icon ? (
                      <img
                        src={tool.icon}
                        alt=""
                        width="22"
                        height="22"
                        loading="lazy"
                        onError={s.imageFallback}
                      />
                    ) : (
                      <span className="tool-initials" aria-hidden="true">
                        {tool.label
                          .split(" ")
                          .slice(0, 2)
                          .map((word) => word[0])
                          .join("")}
                      </span>
                    )}
                    <span className="catalog-tool-copy">
                      <span>{tool.label}</span>
                      {tool.availability === "coming_soon" && (
                        <small className="catalog-availability">
                          {t("Coming soon")}
                        </small>
                      )}
                      {tool.detail && <small>{tool.detail}</small>}
                    </span>
                  </li>
                ))}
              </ul>
            </section>
          );
        })}
        <div className="catalog-footer">
          <a className="link" href={s.siteHref("/integrations")}>
            {t("Tools")} <ArrowUpRight size={17} aria-hidden="true" />
          </a>
          <button
            className="link"
            type="button"
            onClick={(event) => {
              const details = event.currentTarget.closest("details");
              if (details) {
                details.open = false;
                details.querySelector("summary")?.focus();
              }
            }}
          >
            {t("Show fewer tools")} <span aria-hidden="true">−</span>
          </button>
        </div>
      </div>
    </details>
  );
}
