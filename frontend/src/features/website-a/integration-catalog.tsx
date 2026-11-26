import { useState } from "react";
import { Search } from "lucide-react";
import {
  StoryDetails,
  StoryHero,
  StoryLink,
  StoryNext,
  StoryPage,
} from "./story-page";
import { SiteLink } from "./site-link";
import { useWebsite } from "./website-context";
import {
  catalogEntries,
  catalogGroups,
  matchesIntegration,
} from "./nyx-home/catalog-data";
import "./commercial-story.css";

/* The directory is the page: search and category sit directly under a short
   introduction and stay in reach while you scroll the list. */
export function IntegrationCatalog() {
  const s = useWebsite();
  const chinese = s.language === "zh";
  const [query, setQuery] = useState("");
  const [category, setCategory] = useState("");
  const groups = catalogGroups.filter(
    (group) => !category || group.id === category,
  );
  const count = groups
    .flatMap((group) => group.entries)
    .filter((entry) => matchesIntegration(entry, query)).length;
  const categories = [
    { id: "", label: chinese ? "全部" : "All", size: catalogEntries.length },
    ...catalogGroups.map((group) => ({
      id: group.id,
      label: chinese ? group.label_zh : group.label,
      size: group.entries.length,
    })),
  ];

  return (
    <StoryPage className="st-commercial stc-catalog">
      <StoryHero
        title={chinese ? "查找你的智能体可以使用的工具" : "Find the tools your agent can use"}
        description={
          chinese
            ? "连接应用、AI 提供商与语音服务。使用自己的账号或凭据，决定智能体可以执行哪些操作。"
            : "Connect your apps, AI providers and voice services. Use your own accounts or credentials and choose what your agent can do."
        }
      >
        <StoryDetails summary={chinese ? "连接前需要准备什么" : "What you need to connect"}>
          <p>
            {chinese
              ? "连接需要相应账号或凭据及权限。自定义连接还需配置自己的 API 地址。"
              : "An account or credentials and the required permissions are needed. Custom connections also need your own API endpoint."}
          </p>
          <StoryLink href={s.siteHref("/docs/cli/guides/connect-a-service")}>
            {chinese ? "连接指南" : "Connection guide"}
          </StoryLink>
        </StoryDetails>
      </StoryHero>

      <section
        className="stc-directory"
        aria-label={chinese ? "集成目录" : "Integration directory"}
      >
        <div className="st-wrap">
          <div className="stc-filter">
            <label className="stc-search">
              <Search size={18} aria-hidden="true" />
              <input
                id="intQ"
                type="search"
                aria-label={chinese ? "搜索集成" : "Search integrations"}
                placeholder={chinese ? "搜索集成" : "Search integrations"}
                value={query}
                onChange={(event) => setQuery(event.target.value)}
              />
            </label>
            <div
              id="intCat"
              className="stc-cats"
              role="group"
              aria-label={chinese ? "分类" : "Category"}
            >
              {categories.map((item) => (
                <button
                  key={item.id || "all"}
                  type="button"
                  aria-pressed={category === item.id}
                  onClick={() => setCategory(item.id)}
                >
                  {item.label}
                  <span aria-hidden="true">{item.size}</span>
                </button>
              ))}
            </div>
            <p className="stc-count" role="status">
              {chinese
                ? `显示 ${count} / ${catalogEntries.length} 项集成`
                : `Showing ${count} of ${catalogEntries.length} integrations`}
            </p>
          </div>

          {groups.map((group) => {
            const entries = group.entries.filter((entry) =>
              matchesIntegration(entry, query),
            );
            if (!entries.length) return null;
            const label = chinese ? group.label_zh : group.label;
            return (
              <section
                key={group.id}
                className="stc-group"
                aria-labelledby={`int-group-${group.id}`}
              >
                <div className="stc-group-head">
                  <h2 id={`int-group-${group.id}`}>{label}</h2>
                  <span>{entries.length}</span>
                </div>
                <ul className="stc-dir">
                  {entries.map((entry) => (
                    <li
                      className="int"
                      key={entry.id}
                      data-name={entry.label.toLowerCase()}
                      data-cat={group.id}
                    >
                      <span className="int-logo">
                        {entry.icon ? (
                          <img
                            src={entry.icon}
                            alt=""
                            loading="lazy"
                            width="18"
                            height="18"
                            onError={s.imageFallback}
                          />
                        ) : (
                          entry.label.slice(0, 2)
                        )}
                      </span>
                      <span className="int-txt">
                        <b>{entry.label}</b>
                        {entry.availability === "coming_soon" ? (
                          <small className="is-soon">
                            {chinese ? "即将上线" : "Coming soon"}
                          </small>
                        ) : (
                          entry.detail && <small>{entry.detail}</small>
                        )}
                      </span>
                    </li>
                  ))}
                </ul>
              </section>
            );
          })}
          {!count && (
            <p className="stc-empty">
              {chinese
                ? "未找到匹配的集成，请尝试其他名称或分类。"
                : "No matching integrations. Try another name or category."}
            </p>
          )}
          {/* The answer to a missing tool belongs at the end of the list, where it is noticed. */}
          <p className="stc-missing">
            <b>{chinese ? "没有你要的工具？" : "Don’t see your tool?"}</b>{" "}
            {chinese
              ? "把任意 REST API 封装成 MCP 工具，或用自己的 API 地址添加自定义连接。"
              : "Wrap any REST API as MCP tools, or add a custom connection with your own API endpoint."}{" "}
            <SiteLink className="stc-inline-link" href={s.siteHref("/docs/ai/guides/wrap-rest-api-as-mcp")}>
              {chinese ? "阅读封装指南" : "Read the guide"}
            </SiteLink>
          </p>
        </div>
      </section>

      {/* Tools continue to the developer path: one key per agent for the tools above. */}
      <StoryNext
        title={chinese ? "在用 Claude Code、Cursor 或自己的代码？" : "Using Claude Code, Cursor or your own code?"}
        description={
          chinese
            ? "为每个智能体分配独立密钥，让它使用你连接的工具。"
            : "Give each agent its own key to the tools you connect."
        }
        href={s.siteHref("/agents")}
        label={chinese ? "你的智能体" : "Your agent"}
      />
    </StoryPage>
  );
}
