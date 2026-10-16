// Generated from approved A, SHA-256 5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33.
// Run: node scripts/website-a/generate.mjs <baseline.html>
import {
  AudienceSwitcher,
  CodeExample,
  CommunityAccess,
} from "../../interactions";
import { VisualLink, RevealFrame } from "../../visuals/elements";
import { HeroSection, HeroTitle } from "../../visuals/hero";
import { useWebsite } from "../../website-context";
export default function HomePage() {
  const s = useWebsite();
  void s;
  return (
    <>
      {"\n"}
      <HeroSection
        className="hero glowbg"
        style={{ position: "relative", paddingBottom: "56px" }}
      >
        {"\n  "}
        <div className="wrap">
          {"\n    "}
          <HeroTitle />
          {"\n    "}
          <p className="lead">
            {
              "Hand the boring stuff to AI, so you can focus on what matters. Connect your apps, decide what it can touch, and let it get to work."
            }
          </p>
          {"\n    "}
          <AudienceSwitcher
            items={[
              {
                id: "business",
                label: (
                  <>
                    <svg className="i" viewBox="0 0 24 24">
                      <path d="M3 2v7c0 1.1.9 2 2 2h4a2 2 0 0 0 2-2V2"></path>
                      <path d="M7 2v20"></path>
                      <path d="M21 15V2a5 5 0 0 0-5 5v6c0 1.1.9 2 2 2h3Zm0 0v7"></path>
                    </svg>
                    {" For businesses"}
                  </>
                ),
                content: (
                  <>
                    <div className="icp-progress" aria-hidden="true">
                      <span></span>
                    </div>
                    {"\n      "}
                    <div className="icp-copy">
                      {"\n        "}
                      <h2
                        className="h3"
                        style={{ fontSize: "28px", letterSpacing: "-.02em" }}
                      >
                        {"An assistant that just works"}
                      </h2>
                      {"\n        "}
                      <ul
                        className="checks"
                        style={{ marginTop: "20px", gap: "14px" }}
                      >
                        <li style={{ "--i": "0" } as React.CSSProperties}>
                          <span className="ck">
                            <svg className="i" viewBox="0 0 24 24">
                              <path d="M20 6 9 17l-5-5"></path>
                            </svg>
                          </span>
                          <div>
                            <span
                              style={{ color: "var(--fg)", fontSize: "15.5px" }}
                            >
                              {
                                "Answers customers on Telegram and X, day and night"
                              }
                            </span>
                          </div>
                        </li>
                        <li style={{ "--i": "1" } as React.CSSProperties}>
                          <span className="ck">
                            <svg className="i" viewBox="0 0 24 24">
                              <path d="M20 6 9 17l-5-5"></path>
                            </svg>
                          </span>
                          <div>
                            <span
                              style={{ color: "var(--fg)", fontSize: "15.5px" }}
                            >
                              {
                                "Takes bookings and calls customers back for you"
                              }
                            </span>
                          </div>
                        </li>
                        <li style={{ "--i": "2" } as React.CSSProperties}>
                          <span className="ck">
                            <svg className="i" viewBox="0 0 24 24">
                              <path d="M20 6 9 17l-5-5"></path>
                            </svg>
                          </span>
                          <div>
                            <span
                              style={{ color: "var(--fg)", fontSize: "15.5px" }}
                            >
                              {"Ready-made solutions, live in minutes, no code"}
                            </span>
                          </div>
                        </li>
                      </ul>
                      {"\n        "}
                      <div
                        className="sec-cta left"
                        style={{ marginTop: "28px" }}
                      >
                        <VisualLink
                          className="btn p"
                          href={s.authHref("register")}
                          effect="button"
                        >
                          {"Get started"}
                        </VisualLink>
                        <a className="btn" href={s.siteHref("/solutions")}>
                          {"See solutions "}
                          <svg className="i" viewBox="0 0 24 24">
                            <path d="M5 12h14M12 5l7 7-7 7"></path>
                          </svg>
                        </a>
                      </div>
                      {"\n      "}
                    </div>
                    {"\n      "}
                    <div className="icp-visual">
                      <div className="phone">
                        <div className="ph-h">
                          <div className="av">
                            <svg className="i" viewBox="0 0 24 24">
                              <path d="M3 2v7c0 1.1.9 2 2 2h4a2 2 0 0 0 2-2V2"></path>
                              <path d="M7 2v20"></path>
                              <path d="M21 15V2a5 5 0 0 0-5 5v6c0 1.1.9 2 2 2h3Zm0 0v7"></path>
                            </svg>
                          </div>
                          <div>
                            <b style={{ fontSize: "14px" }}>{"Luna Bistro"}</b>
                            <div
                              className="small"
                              style={{ fontSize: "11.5px" }}
                            >
                              {"Telegram · reservations"}
                            </div>
                          </div>
                        </div>
                        <div className="msgs">
                          <div
                            className="m sys"
                            style={
                              {
                                "--i": "1",
                                "--step": "900ms",
                              } as React.CSSProperties
                            }
                          >
                            {"Sat 11:42 PM · restaurant closed"}
                          </div>
                          <div
                            className="m in"
                            style={
                              {
                                "--i": "2",
                                "--step": "900ms",
                              } as React.CSSProperties
                            }
                          >
                            {
                              "Hi, Sarah here! Table for 4 tomorrow at 7:30? One of us is vegan."
                            }
                          </div>
                          <div
                            className="m tool"
                            style={
                              {
                                "--i": "3",
                                "--step": "900ms",
                              } as React.CSSProperties
                            }
                          >
                            {"✓ checked calendar · 7:30 free"}
                          </div>
                          <div
                            className="m tool"
                            style={
                              {
                                "--i": "4",
                                "--step": "900ms",
                              } as React.CSSProperties
                            }
                          >
                            {"✓ booking created · note added"}
                          </div>
                          <div
                            className="m out"
                            style={
                              {
                                "--i": "5",
                                "--step": "900ms",
                              } as React.CSSProperties
                            }
                          >
                            {
                              "Booked! Table for 4, Sun 7:30 PM under Sarah. I’ve noted vegan, so the kitchen knows."
                            }
                          </div>
                        </div>
                      </div>
                    </div>
                    {"\n      "}
                    <div className="icp-try" id="try">
                      <span className="try-tag">{"Connect NyxBot"}</span>
                      <span className="try-txt">
                        {
                          "Sign in to connect your services and set up your assistant."
                        }
                      </span>
                      <span className="try-links">
                        <a className="btn sm" href={s.authHref("register")}>
                          {"Get started"}
                        </a>
                      </span>
                    </div>
                    {"\n    "}
                  </>
                ),
              },
              {
                id: "enthusiast",
                label: (
                  <>
                    <svg className="i" viewBox="0 0 24 24">
                      <path d="M12 3l1.9 5.8L20 11l-6.1 2.2L12 19l-1.9-5.8L4 11l6.1-2.2z"></path>
                    </svg>
                    {" For AI enthusiasts"}
                  </>
                ),
                content: (
                  <>
                    <div className="icp-progress" aria-hidden="true">
                      <span></span>
                    </div>
                    {"\n      "}
                    <div className="icp-copy">
                      {"\n        "}
                      <h2
                        className="h3"
                        style={{ fontSize: "28px", letterSpacing: "-.02em" }}
                      >
                        {"See what your AI can really do"}
                      </h2>
                      {"\n        "}
                      <ul
                        className="checks"
                        style={{ marginTop: "20px", gap: "14px" }}
                      >
                        <li style={{ "--i": "0" } as React.CSSProperties}>
                          <span className="ck">
                            <svg className="i" viewBox="0 0 24 24">
                              <path d="M20 6 9 17l-5-5"></path>
                            </svg>
                          </span>
                          <div>
                            <span
                              style={{ color: "var(--fg)", fontSize: "15.5px" }}
                            >
                              {
                                "Connect Gmail, Calendar, X and more in one message"
                              }
                            </span>
                          </div>
                        </li>
                        <li style={{ "--i": "1" } as React.CSSProperties}>
                          <span className="ck">
                            <svg className="i" viewBox="0 0 24 24">
                              <path d="M20 6 9 17l-5-5"></path>
                            </svg>
                          </span>
                          <div>
                            <span
                              style={{ color: "var(--fg)", fontSize: "15.5px" }}
                            >
                              {"Build a specialist for every job you hand off"}
                            </span>
                          </div>
                        </li>
                        <li style={{ "--i": "2" } as React.CSSProperties}>
                          <span className="ck">
                            <svg className="i" viewBox="0 0 24 24">
                              <path d="M20 6 9 17l-5-5"></path>
                            </svg>
                          </span>
                          <div>
                            <span
                              style={{ color: "var(--fg)", fontSize: "15.5px" }}
                            >
                              {"Let it use a computer when there’s no API"}
                            </span>
                          </div>
                        </li>
                      </ul>
                      {"\n        "}
                      <div
                        className="sec-cta left"
                        style={{ marginTop: "28px" }}
                      >
                        <VisualLink
                          className="btn p"
                          href={s.authHref("register")}
                          effect="button"
                        >
                          {"Get started"}
                        </VisualLink>
                        <a className="btn" href={s.siteHref("/nyxbot")}>
                          {"Meet NyxBot "}
                          <svg className="i" viewBox="0 0 24 24">
                            <path d="M5 12h14M12 5l7 7-7 7"></path>
                          </svg>
                        </a>
                      </div>
                      {"\n      "}
                    </div>
                    {"\n      "}
                    <div className="icp-visual">
                      <div className="phone">
                        <div className="ph-h">
                          <div className="av">
                            <svg className="i" viewBox="0 0 24 24">
                              <path d="M12 3l1.9 5.8L20 11l-6.1 2.2L12 19l-1.9-5.8L4 11l6.1-2.2z"></path>
                            </svg>
                          </div>
                          <div>
                            <b style={{ fontSize: "14px" }}>{"NyxBot"}</b>
                            <div
                              className="small"
                              style={{ fontSize: "11.5px" }}
                            >
                              {"Your assistant"}
                            </div>
                          </div>
                        </div>
                        <div className="msgs">
                          <div
                            className="m in"
                            style={
                              {
                                "--i": "1",
                                "--step": "900ms",
                              } as React.CSSProperties
                            }
                          >
                            {"Summarise my unread Gmail every morning at 8"}
                          </div>
                          <div
                            className="m out"
                            style={
                              {
                                "--i": "2",
                                "--step": "900ms",
                              } as React.CSSProperties
                            }
                          >
                            {
                              "Sure. Connect Gmail first; I’ll only be able to read, not send:"
                            }
                          </div>
                          <div
                            className="m tool"
                            style={
                              {
                                "--i": "3",
                                "--step": "900ms",
                              } as React.CSSProperties
                            }
                          >
                            {"🔗 Secure sign-in link · Gmail (read-only)"}
                          </div>
                          <div
                            className="m tool"
                            style={
                              {
                                "--i": "4",
                                "--step": "900ms",
                              } as React.CSSProperties
                            }
                          >
                            {"✓ Gmail connected · schedule set for 8:00"}
                          </div>
                          <div
                            className="m out"
                            style={
                              {
                                "--i": "5",
                                "--step": "900ms",
                              } as React.CSSProperties
                            }
                          >
                            {"Done. First summary arrives tomorrow at 8."}
                          </div>
                        </div>
                      </div>
                    </div>
                    {"\n      "}
                    <div className="icp-try">
                      <span className="try-tag">{"Connect NyxBot"}</span>
                      <span className="try-txt">
                        {
                          "Sign in to connect your services and set up your assistant."
                        }
                      </span>
                      <span className="try-links">
                        <a className="btn sm" href={s.authHref("register")}>
                          {"Get started"}
                        </a>
                      </span>
                    </div>
                    {"\n    "}
                  </>
                ),
              },
              {
                id: "dev",
                label: (
                  <>
                    <svg className="i" viewBox="0 0 24 24">
                      <polyline points="16 18 22 12 16 6"></polyline>
                      <polyline points="8 6 2 12 8 18"></polyline>
                    </svg>
                    {" For developers"}
                  </>
                ),
                content: (
                  <>
                    <div className="icp-progress" aria-hidden="true">
                      <span></span>
                    </div>
                    {"\n      "}
                    <div className="icp-copy">
                      {"\n        "}
                      <h2
                        className="h3"
                        style={{ fontSize: "28px", letterSpacing: "-.02em" }}
                      >
                        {"Keys for your agents, not passwords"}
                      </h2>
                      {"\n        "}
                      <ul
                        className="checks"
                        style={{ marginTop: "20px", gap: "14px" }}
                      >
                        <li style={{ "--i": "0" } as React.CSSProperties}>
                          <span className="ck">
                            <svg className="i" viewBox="0 0 24 24">
                              <path d="M20 6 9 17l-5-5"></path>
                            </svg>
                          </span>
                          <div>
                            <span
                              style={{ color: "var(--fg)", fontSize: "15.5px" }}
                            >
                              {
                                "MCP and REST for Claude, Cursor, Codex or your own code"
                              }
                            </span>
                          </div>
                        </li>
                        <li style={{ "--i": "1" } as React.CSSProperties}>
                          <span className="ck">
                            <svg className="i" viewBox="0 0 24 24">
                              <path d="M20 6 9 17l-5-5"></path>
                            </svg>
                          </span>
                          <div>
                            <span
                              style={{ color: "var(--fg)", fontSize: "15.5px" }}
                            >
                              {
                                "Scoped keys, approvals and an audit log for every agent"
                              }
                            </span>
                          </div>
                        </li>
                        <li style={{ "--i": "2" } as React.CSSProperties}>
                          <span className="ck">
                            <svg className="i" viewBox="0 0 24 24">
                              <path d="M20 6 9 17l-5-5"></path>
                            </svg>
                          </span>
                          <div>
                            <span
                              style={{ color: "var(--fg)", fontSize: "15.5px" }}
                            >
                              {
                                "Open source under Apache-2.0; self-host with Docker"
                              }
                            </span>
                          </div>
                        </li>
                      </ul>
                      {"\n        "}
                      <div
                        className="sec-cta left"
                        style={{ marginTop: "28px" }}
                      >
                        <VisualLink
                          className="btn p"
                          href={s.authHref("register")}
                          effect="button"
                        >
                          {"Get started"}
                        </VisualLink>
                        <a className="btn" href={s.siteHref("/agents")}>
                          {"Connect your agent "}
                          <svg className="i" viewBox="0 0 24 24">
                            <path d="M5 12h14M12 5l7 7-7 7"></path>
                          </svg>
                        </a>
                      </div>
                      {"\n      "}
                    </div>
                    {"\n      "}
                    <div className="icp-visual">
                      <CodeExample
                        labels={["MCP", "curl"]}
                        snippets={[
                          <>
                            <span className="c">
                              {
                                "// Claude, Cursor, Codex… connect any MCP client to NyxID"
                              }
                            </span>
                            {"\n{\n  "}
                            <span className="s">{'"mcpServers"'}</span>
                            {": {\n    "}
                            <span className="s">{'"nyxid"'}</span>
                            {": { "}
                            <span className="s">{'"url"'}</span>
                            {": "}
                            <span className="s">
                              {'"https://nyx-api.chrono-ai.fun/mcp"'}
                            </span>
                            {" }\n  }\n}\n"}
                            <span className="c">
                              {
                                "// Your connected services appear as tools, scoped to this key."
                              }
                            </span>
                          </>,
                          <>
                            <span className="c">
                              {
                                "# Your agent books a table through NyxID.\n# NyxID injects the credential, checks the scope, logs the call."
                              }
                            </span>
                            {"\ncurl -X POST \\\n  "}
                            <span className="s">
                              {
                                "https://nyx-api.chrono-ai.fun/api/v1/proxy/s/google-calendar/calendars/primary/events"
                              }
                            </span>
                            {" \\\n  -H "}
                            <span className="s">
                              {'"Authorization: Bearer $NYXID_AGENT_KEY"'}
                            </span>
                            {" \\\n  -d "}
                            <span className="s">
                              {
                                '\'{"summary":"Sarah · table for 4","start":{"dateTime":"2026-10-18T19:30:00"}}\''
                              }
                            </span>
                            {"\n\n"}
                            <span className="g">{"→ 200 OK"}</span>
                            {"   "}
                            <span className="c">
                              {"# allowed: calendar create_event"}
                            </span>
                            {"\n"}
                            <span className="k">{"→ 403"}</span>
                            {"      "}
                            <span className="c">
                              {"# if the same key tries delete_event"}
                            </span>
                          </>,
                        ]}
                      />
                    </div>
                    {"\n      "}
                    <div className="icp-try">
                      <span className="try-tag">{"Connect NyxBot"}</span>
                      <span className="try-txt">
                        {
                          "Sign in to connect your services and set up your assistant."
                        }
                      </span>
                      <span className="try-links">
                        <a className="btn sm" href={s.authHref("register")}>
                          {"Get started"}
                        </a>
                      </span>
                    </div>
                    {"\n    "}
                  </>
                ),
              },
            ]}
          />
          {"\n    "}
          {"\n  "}
        </div>
        {"\n"}
      </HeroSection>
      {"\n\n"}
      <section className="s" style={{ paddingTop: "24px" }}>
        {"\n  "}
        <div className="wrap">
          {"\n    "}
          <RevealFrame className="grid g3 doors">
            {"\n      "}
            <VisualLink
              className="card eq door"
              href={s.siteHref("/how-it-works")}
              effect="card"
            >
              <div className="door-k">
                <span className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M12 22v-5M9 8V2M15 8V2M18 8v5a6 6 0 0 1-12 0V8z"></path>
                  </svg>
                </span>
                <span className="over">{"Connect"}</span>
              </div>
              <h3>{"Link the apps you already use"}</h3>
              <p>
                {
                  "Google, Telegram, X, Twilio and 100+ more, through a secure sign-in link. Your passwords stay in the vault."
                }
              </p>
              <div className="door-q">{"“Connect my Google Calendar.”"}</div>
              <span className="more">
                {"How connecting works "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </span>
            </VisualLink>
            <VisualLink
              className="card eq door"
              href={s.siteHref("/security")}
              effect="card"
            >
              <div className="door-k">
                <span className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"></path>
                  </svg>
                </span>
                <span className="over">{"Control"}</span>
              </div>
              <h3>{"Decide what AI may touch"}</h3>
              <p>
                {
                  "Allow, ask first or block, per action. Approve risky steps from your phone and see every call in one log."
                }
              </p>
              <div className="door-q">
                {"“Never let it cancel bookings without me.”"}
              </div>
              <span className="more">
                {"How control works "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </span>
            </VisualLink>
            <VisualLink
              className="card eq door"
              href={s.siteHref("/solutions")}
              effect="card"
            >
              <div className="door-k">
                <span className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <circle cx="12" cy="12" r="10"></circle>
                    <path d="M12 6v6l4 2"></path>
                  </svg>
                </span>
                <span className="over">{"Automate"}</span>
              </div>
              <h3>{"Hand off the busywork"}</h3>
              <p>
                {
                  "Take bookings, answer customers, call people back or send a morning brief, on demand or on a schedule."
                }
              </p>
              <div className="door-q">
                {"“Every weekday at 8, send me today’s bookings.”"}
              </div>
              <span className="more">
                {"See ready-made jobs "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </span>
            </VisualLink>
            {"\n    "}
          </RevealFrame>
          {"\n    "}
          <div className="sec-cta">
            <VisualLink
              className="btn p"
              href={s.authHref("register")}
              effect="button"
            >
              {"Get started"}
            </VisualLink>
            <a className="btn" href={s.siteHref("/how-it-works")}>
              {"Find out more "}
              <svg className="i" viewBox="0 0 24 24">
                <path d="M5 12h14M12 5l7 7-7 7"></path>
              </svg>
            </a>
          </div>
          {"\n  "}
        </div>
        {"\n"}
      </section>
      {"\n\n"}
      <section className="s">
        {"\n  "}
        <div className="wrap split">
          {"\n    "}
          <div>
            <div className="over v">{"Not another chatbot"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"Keep your favourite AI. "}
              <span className="grad">{"Let NyxID handle the access."}</span>
            </h2>
            {"\n      "}
            <p className="lead" style={{ marginTop: "14px" }}>
              {
                "ChatGPT, Claude and Grok are great at answering. NyxBot isn’t trying to replace them. It runs your services: the bookings, replies, files and calls that need real access."
              }
            </p>
            {"\n      "}
            <p className="sub" style={{ marginTop: "14px" }}>
              {
                "Already use one of them, or build your own agent? Connect it to NyxID and it gets the same scoped, approved, logged access."
              }
            </p>
            {"\n      "}
            <div style={{ marginTop: "24px" }}>
              <div className="sec-cta left">
                <VisualLink
                  className="btn p"
                  href={s.authHref("register")}
                  effect="button"
                >
                  {"Get started"}
                </VisualLink>
                <a className="btn" href={s.siteHref("/agents")}>
                  {"Find out more "}
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M5 12h14M12 5l7 7-7 7"></path>
                  </svg>
                </a>
              </div>
            </div>
          </div>
          {"\n    "}
          <div className="fit fit-v">
            {"\n      "}
            <div className="fit-n ">
              <span className="ic ">
                <svg className="i" viewBox="0 0 24 24">
                  <circle cx="12" cy="8" r="4"></circle>
                  <path d="M4 21v-1a6 6 0 0 1 6-6h4a6 6 0 0 1 6 6v1"></path>
                </svg>
              </span>
              <div>
                <b>{"You, in plain words"}</b>
                <span>{"or your own AI agent"}</span>
              </div>
            </div>
            <span className="fit-a">
              <svg className="i" viewBox="0 0 24 24">
                <path d="M5 12h14M12 5l7 7-7 7"></path>
              </svg>
            </span>
            <div className="fit-n ">
              <span className="ic ">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M12 3l1.9 5.8L20 11l-6.1 2.2L12 19l-1.9-5.8L4 11l6.1-2.2z"></path>
                </svg>
              </span>
              <div>
                <b>{"NyxBot"}</b>
                <span>{"turns the ask into steps"}</span>
              </div>
            </div>
            <span className="fit-a">
              <svg className="i" viewBox="0 0 24 24">
                <path d="M5 12h14M12 5l7 7-7 7"></path>
              </svg>
            </span>
            <div className="fit-n hi">
              <span className="ic g">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"></path>
                </svg>
              </span>
              <div>
                <b>{"NyxID"}</b>
                <span>{"checks the key, asks you, logs it"}</span>
              </div>
            </div>
            <span className="fit-a">
              <svg className="i" viewBox="0 0 24 24">
                <path d="M5 12h14M12 5l7 7-7 7"></path>
              </svg>
            </span>
            <div className="fit-n ">
              <span className="ic ">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M12 22v-5M9 8V2M15 8V2M18 8v5a6 6 0 0 1-12 0V8z"></path>
                </svg>
              </span>
              <div>
                <b>{"Your apps & APIs"}</b>
                <span>{"100+ services"}</span>
              </div>
            </div>
            {"\n    "}
          </div>
          {"\n  "}
        </div>
        {"\n"}
      </section>
      {"\n\n"}
      <section className="s glowbg">
        {"\n  "}
        <div className="wrap">
          {"\n    "}
          <div
            className="center"
            style={{
              textAlign: "center",
              maxWidth: "720px",
              marginBottom: "36px",
            }}
          >
            <div className="over v">{"Integrations"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"Works with the tools you already use"}
            </h2>
            <p className="lead center" style={{ marginTop: "14px" }}>
              {
                "Connect them once. NyxBot and your own agents get scoped access, never your passwords."
              }
            </p>
          </div>
          {"\n    "}
          <div className="int-wall">
            <div
              className="int "
              data-name="notion"
              data-cat="0"
              data-av="1"
              hidden={!s.integrationVisible("notion", "0", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=notion.so&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Notion"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="slack"
              data-cat="1"
              data-av="1"
              hidden={!s.integrationVisible("slack", "1", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=slack.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Slack"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="github"
              data-cat="2"
              data-av="1"
              hidden={!s.integrationVisible("github", "2", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=github.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"GitHub"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="linear"
              data-cat="2"
              data-av="1"
              hidden={!s.integrationVisible("linear", "2", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=linear.app&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Linear"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="google sheets"
              data-cat="3"
              data-av="1"
              hidden={!s.integrationVisible("google sheets", "3", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=google.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Google Sheets"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="gmail"
              data-cat="4"
              data-av="1"
              hidden={!s.integrationVisible("gmail", "4", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=mail.google.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Gmail"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="google calendar"
              data-cat="5"
              data-av="1"
              hidden={!s.integrationVisible("google calendar", "5", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=calendar.google.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Google Calendar"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="google drive"
              data-cat="6"
              data-av="1"
              hidden={!s.integrationVisible("google drive", "6", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=drive.google.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Google Drive"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="asana"
              data-cat="7"
              data-av="1"
              hidden={!s.integrationVisible("asana", "7", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=asana.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Asana"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="box"
              data-cat="6"
              data-av="1"
              hidden={!s.integrationVisible("box", "6", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=box.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Box"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="jira"
              data-cat="2"
              data-av="1"
              hidden={!s.integrationVisible("jira", "2", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=atlassian.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Jira"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="zoom"
              data-cat="9"
              data-av="1"
              hidden={!s.integrationVisible("zoom", "9", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=zoom.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Zoom"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="dropbox"
              data-cat="6"
              data-av="1"
              hidden={!s.integrationVisible("dropbox", "6", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=dropbox.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Dropbox"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="hubspot"
              data-cat="8"
              data-av="1"
              hidden={!s.integrationVisible("hubspot", "8", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=hubspot.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"HubSpot"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="clickup"
              data-cat="7"
              data-av="1"
              hidden={!s.integrationVisible("clickup", "7", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=clickup.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"ClickUp"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="google docs"
              data-cat="0"
              data-av="1"
              hidden={!s.integrationVisible("google docs", "0", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=docs.google.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Google Docs"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="airtable"
              data-cat="3"
              data-av="1"
              hidden={!s.integrationVisible("airtable", "3", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=airtable.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Airtable"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="microsoft outlook (mail & calendar)"
              data-cat="4"
              data-av="1"
              hidden={
                !s.integrationVisible(
                  "microsoft outlook (mail & calendar)",
                  "4",
                  "1",
                )
              }
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=learn.microsoft.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Microsoft Outlook (Mail & Calendar)"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="stripe"
              data-cat="11"
              data-av="1"
              hidden={!s.integrationVisible("stripe", "11", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=stripe.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Stripe"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="microsoft onedrive"
              data-cat="6"
              data-av="1"
              hidden={!s.integrationVisible("microsoft onedrive", "6", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=onedrive.live.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Microsoft OneDrive"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="calendly"
              data-cat="5"
              data-av="1"
              hidden={!s.integrationVisible("calendly", "5", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=calendly.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Calendly"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="figma"
              data-cat="12"
              data-av="1"
              hidden={!s.integrationVisible("figma", "12", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=figma.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Figma"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="microsoft teams"
              data-cat="1"
              data-av="1"
              hidden={!s.integrationVisible("microsoft teams", "1", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=teams.microsoft.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Microsoft Teams"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="intercom"
              data-cat="10"
              data-av="1"
              hidden={!s.integrationVisible("intercom", "10", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=intercom.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Intercom"}</b>
              </span>
            </div>
          </div>
          {"\n    "}
          <p
            className="small center"
            style={{ textAlign: "center", marginTop: "18px" }}
          >
            {"56 live today · 89 coming soon"}
          </p>
          {"\n    "}
          <div className="sec-cta">
            <VisualLink
              className="btn p"
              href={s.authHref("register")}
              effect="button"
            >
              {"Get started"}
            </VisualLink>
            <a className="btn" href={s.siteHref("/integrations")}>
              {"See all integrations "}
              <svg className="i" viewBox="0 0 24 24">
                <path d="M5 12h14M12 5l7 7-7 7"></path>
              </svg>
            </a>
          </div>
          {"\n  "}
        </div>
        {"\n"}
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap">
          <div className="band">
            <h2 className="h2">{"Start with one job today."}</h2>
            <p style={{ opacity: ".88", marginTop: "12px", fontSize: "17px" }}>
              {"Platform prices, no markup. Free to self-host."}
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
                {"Get started"}
              </a>
              <a className="btn lg gh" href={s.siteHref("/solutions")}>
                {"Browse solutions"}
              </a>
            </div>
          </div>
        </div>
      </section>
      <CommunityAccess />
    </>
  );
}
