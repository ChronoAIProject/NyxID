import { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { Sparkles, Utensils, Newspaper, Monitor, Phone } from "lucide-react";
import { useWebsite } from "./website-context";
import enDemos from "./generated/en-demos.json";
import zhDemos from "./generated/zh-demos.json";
import enFilm from "./generated/en-film.json";
import zhFilm from "./generated/zh-film.json";
import "./workflow-film.css";

function useReducedMotion() {
  const [reduced, setReduced] = useState(
    () => window.matchMedia("(prefers-reduced-motion: reduce)").matches,
  );
  useEffect(() => {
    const media = window.matchMedia("(prefers-reduced-motion: reduce)");
    const update = () => setReduced(media.matches);
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
  }, []);
  return reduced;
}

export function AudienceSwitcher({
  items,
}: {
  items: { id: string; label: ReactNode; content: ReactNode }[];
}) {
  const initial = new URLSearchParams(window.location.search).get("for");
  const [selected, setSelected] = useState(() =>
    Math.max(
      0,
      items.findIndex((item) => item.id === initial),
    ),
  );
  const [stopped, setStopped] = useState(Boolean(initial));
  const paused = useRef(false);
  const visible = useRef(true);
  const root = useRef<HTMLDivElement>(null);
  const tabs = useRef<(HTMLButtonElement | null)[]>([]);
  const reduced = useReducedMotion();
  useEffect(() => {
    const observer = new IntersectionObserver(
      ([entry]) => {
        visible.current = entry?.isIntersecting ?? false;
      },
      { threshold: 0.2 },
    );
    if (root.current) observer.observe(root.current);
    return () => observer.disconnect();
  }, []);
  useEffect(() => {
    if (stopped || reduced) return;
    let remaining = 9000;
    let previous = Date.now();
    const timer = window.setInterval(() => {
      const now = Date.now();
      if (!paused.current && visible.current && !document.hidden)
        remaining -= now - previous;
      previous = now;
      if (remaining <= 0) {
        remaining = 9000;
        setSelected((index) => (index + 1) % items.length);
      }
    }, 200);
    return () => window.clearInterval(timer);
  }, [stopped, reduced, items.length]);
  const select = (index: number) => {
    setStopped(true);
    setSelected(index);
  };
  return (
    <div
      ref={root}
      className="audience-switcher"
      onMouseEnter={() => {
        paused.current = true;
      }}
      onMouseLeave={() => {
        paused.current = false;
      }}
      onFocusCapture={() => setStopped(true)}
    >
      <div
        className="icp-tabs"
        role="tablist"
        aria-label={useWebsite().language === "zh" ? "适合谁" : "Who it’s for"}
      >
        {items.map((item, index) => (
          <button
            key={item.id}
            ref={(element) => {
              tabs.current[index] = element;
            }}
            type="button"
            role="tab"
            id={`icp-t-${item.id}`}
            aria-controls={`icp-p-${item.id}`}
            aria-selected={selected === index}
            tabIndex={selected === index ? 0 : -1}
            onClick={() => select(index)}
            onKeyDown={(event) => {
              if (
                !["ArrowRight", "ArrowLeft", "Home", "End"].includes(event.key)
              )
                return;
              event.preventDefault();
              const next =
                event.key === "Home"
                  ? 0
                  : event.key === "End"
                    ? items.length - 1
                    : (index +
                        (event.key === "ArrowRight" ? 1 : items.length - 1)) %
                      items.length;
              select(next);
              tabs.current[next]?.focus();
            }}
          >
            {item.label}
          </button>
        ))}
      </div>
      {items.map((item, index) => (
        <div
          key={`${item.id}-${selected === index}`}
          role="tabpanel"
          id={`icp-p-${item.id}`}
          aria-labelledby={`icp-t-${item.id}`}
          hidden={selected !== index}
          className={`icp-panel play ${stopped || reduced ? "is-stopped" : "is-running"}`}
        >
          {item.content}
        </div>
      ))}
    </div>
  );
}

export function CodeExample({
  labels,
  snippets,
}: {
  labels: string[];
  snippets: ReactNode[];
}) {
  const [selected, setSelected] = useState(0);
  return (
    <div className="code">
      <div className="bar">
        <i />
        <i />
        <i />
        <div className="tabs">
          {labels.map((label, index) => (
            <button
              type="button"
              key={label}
              className={index === selected ? "on" : ""}
              aria-pressed={index === selected}
              onClick={() => setSelected(index)}
            >
              {label}
            </button>
          ))}
        </div>
      </div>
      {snippets.map((snippet, index) => (
        <pre hidden={selected !== index} key={index}>
          {snippet}
        </pre>
      ))}
    </div>
  );
}

export function DemoChips() {
  const s = useWebsite();
  const data = s.language === "zh" ? zhDemos : enDemos;
  return (
    <div
      className="pills"
      id="nbChips"
      onMouseEnter={s.stopDemo}
      onFocusCapture={s.stopDemo}
    >
      {data.demos.map((demo, index) => (
        <button
          type="button"
          className={`pill ${s.demoIndex === index ? "on" : ""}`}
          aria-pressed={s.demoIndex === index}
          key={demo.label}
          onClick={() => s.selectDemo(index)}
        >
          {demo.label}
        </button>
      ))}
    </div>
  );
}
export function DemoAgents() {
  const s = useWebsite();
  const data = s.language === "zh" ? zhDemos : enDemos;
  const icons = [Utensils, Newspaper, Monitor, Phone];
  return (
    <aside className="nb-side" id="nbSide">
      <div className="nb-sl">{s.language === "zh" ? "助手" : "Agents"}</div>
      <div className="nb-ag on">
        <span className="nb-av sm">
          <Sparkles size={16} />
        </span>
        <span>NyxBot</span>
      </div>
      {data.agents.map((agent, index) => {
        const Icon = icons[index] ?? Sparkles;
        const active = data.demos[s.demoIndex]!.active === index;
        return (
          <div className={`nb-ag ${active ? "new" : ""}`} key={agent.name}>
            <span className="ic">
              <Icon size={16} />
            </span>
            <span>{agent.name}</span>
            {active && (
              <span className="badge b-v">
                {s.language === "zh" ? "新" : "new"}
              </span>
            )}
          </div>
        );
      })}
    </aside>
  );
}
export function DemoMessages() {
  const s = useWebsite();
  const data = s.language === "zh" ? zhDemos : enDemos;
  return (
    <div
      className="msgs nb-msgs"
      id="nbMsgs"
      onMouseEnter={s.stopDemo}
      onFocusCapture={s.stopDemo}
    >
      {data.demos[s.demoIndex]!.messages.map((message, index) => (
        <div className={`m ${message.kind}`} key={`${s.demoIndex}-${index}`}>
          {message.text}
        </div>
      ))}
    </div>
  );
}

export function SolutionFilm() {
  const s = useWebsite();
  const [index, setIndex] = useState(0);
  const [paused, setPaused] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const visible = useRef(true);
  const reduced = useReducedMotion();
  const data = s.language === "zh" ? zhFilm : enFilm;
  useEffect(() => {
    const observer = new IntersectionObserver(([entry]) => {
      visible.current = entry?.isIntersecting ?? false;
    });
    if (root.current) observer.observe(root.current);
    return () => observer.disconnect();
  }, []);
  useEffect(() => {
    if (paused || reduced) return;
    const timer = window.setInterval(() => {
      if (visible.current && !document.hidden)
        setIndex((i) => (i + 1) % data.length);
    }, 6600);
    return () => window.clearInterval(timer);
  }, [paused, reduced, data.length]);
  const scene = data[index]!;
  const permission: Record<string, string> =
    s.language === "zh"
      ? { ok: "允许", ro: "只读", ask: "先问我", no: "禁止" }
      : {
          ok: "Allowed",
          ro: "Read-only",
          ask: "Ask me first",
          no: "Not allowed",
        };
  return (
    <div
      ref={root}
      className="workflow-film"
      onFocusCapture={(event) => {
        if (!event.target.closest("[data-playback-control]")) setPaused(true);
      }}
    >
      <div className="workflow-heading">
        <div>
          <div className="over">
            NyxID ·{" "}
            {s.language === "zh"
              ? "安全的智能体网关"
              : "The secure agent gateway"}
          </div>
          <h3>
            {s.language === "zh"
              ? "NyxBot：一个助手，胜任各种工作。"
              : "NyxBot: one assistant, any job."}
          </h3>
        </div>
        <button
          data-playback-control
          type="button"
          className="btn sm"
          aria-pressed={paused}
          onClick={() => setPaused((p) => !p)}
        >
          {paused
            ? s.language === "zh"
              ? "播放"
              : "Play"
            : s.language === "zh"
              ? "暂停"
              : "Pause"}
        </button>
      </div>
      <p className="demo-label">
        {s.language === "zh" ? "工作方式示例" : "Illustrative workflow"}
      </p>
      <div className="workflow-chips">
        {data.map((demo, i) => (
          <button
            key={demo.id}
            type="button"
            className={`pill ${index === i ? "on" : ""}`}
            aria-pressed={index === i}
            onClick={() => {
              setPaused(true);
              setIndex(i);
            }}
          >
            {demo.label}
          </button>
        ))}
      </div>
      <div className="workflow-columns" key={index}>
        <div>
          <div className="workflow-label">
            1 · {s.language === "zh" ? "在哪里聊天" : "Where you chat"}
          </div>
          <div className="phone">
            <div className="ph-h">
              <b>{scene.bot}</b>
              <span className="small">Telegram · {scene.sub}</span>
            </div>
            <div className="msgs">
              <div className="m sys">{scene.sys}</div>
              {scene.user && <div className="m in">{scene.user}</div>}
              {scene.tools.map((tool) => (
                <div className="m tool" key={tool}>
                  {tool}
                </div>
              ))}
              <div className="m out">{scene.reply}</div>
            </div>
          </div>
        </div>
        <div>
          <div className="workflow-label">
            2 ·{" "}
            {s.language === "zh"
              ? "NyxID 允许它用什么"
              : "What NyxID lets it use"}
          </div>
          <div className="perm-card">
            <div className="pc-h">
              <div>
                <b>NyxID</b>
                <div className="small">{scene.key}</div>
              </div>
              <span className="badge b-v">
                {s.language === "zh" ? "限权密钥" : "Scoped key"}
              </span>
            </div>
            {scene.rows.map(([glyph, name, action, access], i) => (
              <div className="prow" key={i}>
                <span className="gl">{String(glyph).toUpperCase()}</span>
                <div>
                  {name}
                  <small>{action}</small>
                </div>
                <span className={`badge b-${access}`}>
                  {permission[String(access)]}
                </span>
              </div>
            ))}
            <p className="small">
              {s.language === "zh"
                ? "交给 AI 的密码：0 · 每次调用均有记录"
                : "Passwords shared with AI: 0 · Every call logged"}
            </p>
          </div>
        </div>
        <div>
          <div className="workflow-label">
            3 ·{" "}
            {s.language === "zh"
              ? "或从你的应用调用"
              : "Or call it from your app"}
          </div>
          <div className="code">
            <div className="bar">
              <i />
              <i />
              <i />
              <span className="small">your-app.sh · REST / MCP</span>
            </div>
            <pre>{scene.req}</pre>
            <pre className="workflow-response">{scene.res}</pre>
          </div>
        </div>
      </div>
      <p className="workflow-caption">{scene.cap}</p>
    </div>
  );
}

export function CommunityAccess() {
  const s = useWebsite();
  useEffect(() => {
    if (window.location.hash === "#beta") {
      document
        .getElementById("beta")
        ?.scrollIntoView({
          behavior: window.matchMedia("(prefers-reduced-motion: reduce)")
            .matches
            ? "auto"
            : "smooth",
        });
    }
  }, []);
  return (
    <section className="s" id="beta">
      <div className="wrap">
        <div className="community">
          <div>
            <div className="over">Early access</div>
            <h2 className="h2">
              {s.language === "zh"
                ? "与我们一同步入暗夜"
                : "Step into the night with us"}
            </h2>
            <p className="sub" style={{ marginTop: 16 }}>
              {s.language === "zh"
                ? "NyxID 目前处于封闭内测。加入我们的 Discord，名额开放时团队将向你发送邀请码。"
                : "NyxID is in closed beta. Join our Discord; the team will share an invitation when spots open."}
            </p>
          </div>
          <a
            className="btn p lg"
            href="https://discord.gg/QMvcs8UQBW"
            target="_blank"
            rel="noopener noreferrer"
          >
            {s.language === "zh" ? "加入 Discord →" : "Join Discord →"}
          </a>
        </div>
      </div>
    </section>
  );
}
