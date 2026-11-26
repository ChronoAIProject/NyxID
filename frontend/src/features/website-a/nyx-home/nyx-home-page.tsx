// NyxID home page, ported from the "light night" design (mockups/landing-combined/nyxid-light-night.html).
// The markup below is the design's markup converted to JSX; nyx-home-motion.js runs its animations.
import { useLayoutEffect, useRef } from "react";
import type { CSSProperties } from "react";
import { useWebsite } from "../website-context";
import { HomeJobs } from "./nyx-home-jobs";
import { ToolCatalog } from "./nyx-home-tools";
import { PublicHeader, PublicFooter } from "../../public-site/public-chrome";
import { usePublicThemeElement } from "../../public-site/public-theme";
import "./nyx-home.css";

export default function NyxHomePage() {
  const s = useWebsite();
  const ref = useRef<HTMLDivElement>(null);
  usePublicThemeElement(ref);
  // Mark entrance animations before paint; theme is shared with the detailed pages.
  useLayoutEffect(() => {
    ref.current?.classList.add("js");
  }, []);
  // The animation code (GSAP, Lenis, WebGL) loads only in the browser, after the page shows.
  useLayoutEffect(() => {
    const page = ref.current;
    if (!page) return;
    let stop: (() => void) | undefined;
    let left = false;
    import("./nyx-home-motion")
      .then(({ startNyxHome }) => {
        if (!left) stop = startNyxHome(page);
      })
      .catch((error: unknown) => {
        // The page stays readable without motion; report why the animations did not start.
        console.error("NyxID home animations failed to start", error);
      });
    return () => {
      left = true;
      stop?.();
    };
  }, []);
  return (
    <div ref={ref} className="nyx-home nyx-public">
      {" "}
      <div className="page-sky" aria-hidden="true" />{" "}
      <a className="skip" href="#main">
        Skip to content
      </a>{" "}
      <svg
        width="0"
        height="0"
        style={{ position: "absolute" } as CSSProperties}
        aria-hidden="true"
      >
        {" "}
        <symbol id="ck" viewBox="0 0 12 12">
          <path
            d="M2 6.4l2.6 2.6L10 3.4"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </symbol>{" "}
        <symbol id="hand" viewBox="0 0 12 12">
          <path
            d="M6 2.2v4.3M6 9.3v.4"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
          />
        </symbol>{" "}
        <symbol id="no" viewBox="0 0 12 12">
          <path
            d="M3 3l6 6M9 3l-6 6"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
          />
        </symbol>{" "}
        <symbol id="lock" viewBox="0 0 16 16">
          <path
            d="M4.5 7V5a3.5 3.5 0 017 0v2M3.5 7h9v6.5h-9z"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinejoin="round"
          />
        </symbol>{" "}
      </svg>{" "}
      <div hidden id="kinds">
        {" "}
        <span data-k="call">
          <span className="shape s-call">
            <span className="o-l">Telegram</span>
            <span className="o-m">Table for 4 at 7:30</span>
            <span className="o-s">Sarah, one vegan</span>
          </span>
        </span>{" "}
        <span data-k="cal">
          <span className="shape s-cal">
            <span className="cal-top">Every weekday</span>
            <span className="cal-big">08:00</span>
            <span className="o-s">Morning brief</span>
          </span>
        </span>{" "}
        <span data-k="letter">
          <span className="shape s-letter">
            <svg viewBox="0 0 100 66" preserveAspectRatio="none">
              <path d="M0 0 L50 34 L100 0" />
            </svg>
            <span className="o-l">X</span>
            <span className="o-m">Open on Monday?</span>
            <span className="o-s">A DM to answer</span>
          </span>
        </span>{" "}
        <span data-k="sticky">
          <span className="shape s-sticky">
            <span className="o-m">Call Mr Tan back</span>
            <span className="o-s">Missed call, 6:10 PM</span>
          </span>
        </span>{" "}
        <span data-k="invoice">
          <span className="shape s-invoice">
            <span className="o-l">Stripe</span>
            <span className="o-m">Refund $40</span>
            <span className="o-s">Ask first</span>
          </span>
        </span>{" "}
        <span data-k="slip">
          <span className="shape s-slip">
            <span className="o-l">Claude Code</span>
            <span className="o-m">Open a pull request</span>
            <span className="o-s">GitHub</span>
          </span>
        </span>{" "}
        <span data-k="receipt">
          <span className="shape s-receipt">
            <span className="o-l">Google Sheets</span>
            <span className="row">
              <span>Takings</span>
              <b>Sat</b>
            </span>
            <span className="row">
              <span>Row</span>
              <b>212</b>
            </span>
            <span className="row">
              <span>Rule</span>
              <b>Allow</b>
            </span>
          </span>
        </span>{" "}
      </div>{" "}
      <PublicHeader home siteHref={s.siteHref} authHref={s.authHref} localeHref={s.localeHref} />
      <nav className="hud" aria-label="Page position" data-hud="">
        {" "}
        <a href="#top">
          <i />
          <span>01 Ask</span>
        </a>{" "}
        <a href="#pile">
          <i />
          <span>02 NyxBot</span>
        </a>{" "}
        <a href="#agent">
          <i />
          <span>03 Your agent</span>
        </a>{" "}
        <a href="#apps">
          <i />
          <span>04 Tools</span>
        </a>{" "}
        <a href="#jobs">
          <i />
          <span>05 Solutions</span>
        </a>{" "}
        <a href="#end">
          <i />
          <span>06 Start</span>
        </a>{" "}
        <output className="hud-y" data-hud-y="" aria-hidden="true">
          000
        </output>{" "}
      </nav>{" "}
      <main id="main">
        {" "}
        {/* ============ 1. HERO ============ */}{" "}
        <section className="hero" id="top" data-hero="">
          {" "}
          <div className="sky" aria-hidden="true" />{" "}
          <div className="rain" data-rain="" aria-hidden="true" />{" "}
          <div className="wrap hero-grid">
            {" "}
            <div className="hero-copy">
              {" "}
              <h1 data-in="" style={{ "--d": "0" } as CSSProperties}>
                <span>The AI assistant</span>
                <span>
                  {"that gets it "}
                  <em>done.</em>
                </span>
              </h1>{" "}
              <p
                className="lede"
                data-in=""
                style={{ "--d": "1" } as CSSProperties}
              >
                Ask NyxBot in plain words on Telegram, X or the NyxID app. It
                books the table, replies to the customer and sends the morning
                brief, under rules you set.
              </p>{" "}
              <div
                className="hero-cta"
                data-in=""
                style={{ "--d": "2" } as CSSProperties}
              >
                {" "}
                <a className="btn" href={s.authHref("register")}>
                  {"Get started "}
                  <span className="arr" aria-hidden="true">
                    →
                  </span>
                </a>{" "}
                <a
                  className="link"
                  href="https://github.com/ChronoAIProject/NyxID"
                >
                  {"Self-host with Docker "}
                  <span className="arr" aria-hidden="true">
                    →
                  </span>
                </a>{" "}
              </div>{" "}
              <p
                className="hero-facts"
                data-in=""
                style={{ "--d": "3" } as CSSProperties}
              >
                <span>Open source under Apache-2.0</span>
                <span>
                  <img src="/landing/nyx/icons/telegram.svg" alt="Telegram" />
                  <img src="/landing/nyx/icons/x.svg" alt="X" />
                  <img
                    src="/landing/nyx/icons/whatsapp.svg"
                    alt="WhatsApp (coming soon)"
                    title="WhatsApp — coming soon"
                  />
                  <img src="/landing/nyx/icons/slack.svg" alt="Slack" />
                  <img src="/landing/nyx/icons/discord.svg" alt="Discord" />
                  and the NyxID app
                </span>
              </p>{" "}
            </div>{" "}
            <div
              className="nstage"
              data-nstage=""
              data-in=""
              style={{ "--d": "1" } as CSSProperties}
              aria-hidden="true"
            >
              {" "}
              <svg className="ring" viewBox="0 0 100 100">
                <line className="x" x1="50" y1="2" x2="50" y2="98" />
                <line className="x" x1="2" y1="50" x2="98" y2="50" />
                <circle className="c" cx="50" cy="50" r="41" />
                <circle className="t" data-ticks="" cx="50" cy="50" r="45" />
                <circle
                  className="arc"
                  pathLength="100"
                  cx="50"
                  cy="50"
                  r="45"
                />
                <circle className="burst" cx="50" cy="50" r="45" />
              </svg>{" "}
              <div className="nmark" data-mark="">
                <canvas />
                <div className="nmark-fallback" />
              </div>{" "}
              <p className="n-count">
                <b data-count="">0</b>
                {" of "}
                <span data-total="">10</span>
                {" requests handled"}
              </p>{" "}
              <div className="done-chat" data-done-chat="">
                <p className="done-bubble">
                  <span className="done-dots">
                    <i />
                    <i />
                    <i />
                  </span>
                  <span className="done-say">
                    <i>
                      <svg>
                        <use href="#ck" />
                      </svg>
                    </i>
                    Done!
                  </span>
                </p>
              </div>{" "}
            </div>{" "}
          </div>{" "}
        </section>{" "}
        {/* ============ 2. HOW IT WORKS: the pile on the counter (landing-2; kit.js fills this section) ============ */}{" "}
        <section
          className="pile-sec"
          id="pile"
          aria-label="How NyxBot works through the night's pile"
        >
          {" "}
          <div className="wrap pile-grid">
            {" "}
            <div className="pile-copy">
              {" "}
              <p className="section-label">NyxBot at work</p>
              <h2>The pile on your counter, sorted before you wake.</h2>{" "}
              <p className="lede">
                Your agent works in the apps and chats you connect. You set the
                limits and review actions that need approval.
              </p>{" "}
              <div className="pile-detail-links">
                <a className="link" href={s.siteHref("/how-it-works")}>
                  How it works{" "}
                  <span className="arr" aria-hidden="true">
                    →
                  </span>
                </a>
                <a className="link" href={s.siteHref("/security")}>
                  Security{" "}
                  <span className="arr" aria-hidden="true">
                    →
                  </span>
                </a>
              </div>
              <p className="count" aria-live="polite">
                <span>
                  <b data-n-done="">0</b>
                  done
                </span>
                <span>
                  <b data-n-wait="">0</b>
                  waiting for approval
                </span>
              </p>{" "}
              <div
                className="pile"
                data-pile=""
                aria-label="Things on the counter"
              >
                {" "}
                <p className="pile-empty">The counter is clear.</p>{" "}
                <button
                  className="obj"
                  type="button"
                  data-obj="slip"
                  data-dest="done"
                  style={
                    {
                      "--x": "2%",
                      "--y": "4%",
                      "--r": "-7deg",
                      "--z": "2",
                    } as CSSProperties
                  }
                  aria-label="Reservation from Sarah. Hand to NyxBot."
                >
                  {" "}
                  <span className="obj-in">
                    <span className="shape s-slip">
                      <span className="o-l">Reservation</span>
                      <span className="o-m">Table for 4</span>
                      <span className="o-s">Sun 7:30 PM, Sarah</span>
                    </span>
                    <svg
                      className="stamp"
                      viewBox="0 0 40 40"
                      aria-hidden="true"
                    >
                      <circle cx="20" cy="20" r="17" />
                      <path d="M12 20.5l5.5 5.5L28.5 14" />
                    </svg>
                  </span>
                </button>{" "}
                <button
                  className="obj"
                  type="button"
                  data-obj="letter"
                  data-dest="done"
                  style={
                    {
                      "--x": "45%",
                      "--y": "0%",
                      "--r": "5deg",
                      "--z": "1",
                    } as CSSProperties
                  }
                  aria-label="Question on X from maya_eats. Hand to NyxBot."
                >
                  {" "}
                  <span className="obj-in">
                    <span className="shape s-letter">
                      <svg
                        viewBox="0 0 100 66"
                        preserveAspectRatio="none"
                        aria-hidden="true"
                      >
                        <path d="M0 0 L50 34 L100 0" />
                      </svg>
                      <span className="o-l">
                        <img src="/landing/nyx/icons/x.svg" alt="" />
                        @maya_eats
                      </span>
                      <span className="o-m">Gluten-free dishes?</span>
                      <span className="o-s">Open on Monday?</span>
                    </span>
                    <svg
                      className="stamp"
                      viewBox="0 0 40 40"
                      aria-hidden="true"
                    >
                      <circle cx="20" cy="20" r="17" />
                      <path d="M12 20.5l5.5 5.5L28.5 14" />
                    </svg>
                  </span>
                </button>{" "}
                <button
                  className="obj"
                  type="button"
                  data-obj="invoice"
                  data-dest="wait"
                  style={
                    {
                      "--x": "68%",
                      "--y": "30%",
                      "--r": "8deg",
                      "--z": "3",
                    } as CSSProperties
                  }
                  aria-label="Invoice from Harbour Fish Co. Hand to NyxBot."
                >
                  {" "}
                  <span className="obj-in">
                    <svg
                      className="clip"
                      viewBox="0 0 12 26"
                      aria-hidden="true"
                    >
                      <path d="M3 8v11a3 3 0 006 0V5a2 2 0 00-4 0v12" />
                    </svg>
                    <span className="shape s-invoice">
                      <span className="o-l">Harbour Fish Co.</span>
                      <span className="o-m">Friday order</span>
                      <span className="o-s">Price up 8%. Confirm?</span>
                    </span>
                    <svg
                      className="stamp"
                      viewBox="0 0 40 40"
                      aria-hidden="true"
                    >
                      <circle cx="20" cy="20" r="17" />
                      <path d="M20 11v11M20 28v.5" />
                    </svg>
                  </span>
                </button>{" "}
                <button
                  className="obj"
                  type="button"
                  data-obj="receipt"
                  data-dest="done"
                  style={
                    {
                      "--x": "49%",
                      "--y": "47%",
                      "--r": "-9deg",
                      "--z": "2",
                    } as CSSProperties
                  }
                  aria-label="Till receipt for Saturday. Hand to NyxBot."
                >
                  {" "}
                  <span className="obj-in">
                    <span className="shape s-receipt">
                      <span className="o-l">Saturday till</span>
                      <span className="row">
                        <span>Tables</span>
                        <b>38</b>
                      </span>
                      <span className="row">
                        <span>Card</span>
                        <b>31</b>
                      </span>
                      <span className="row">
                        <span>Cash</span>
                        <b>7</b>
                      </span>
                    </span>
                    <svg
                      className="stamp"
                      viewBox="0 0 40 40"
                      aria-hidden="true"
                    >
                      <circle cx="20" cy="20" r="17" />
                      <path d="M12 20.5l5.5 5.5L28.5 14" />
                    </svg>
                  </span>
                </button>{" "}
                <button
                  className="obj"
                  type="button"
                  data-obj="sticky"
                  data-dest="done"
                  style={
                    {
                      "--x": "27%",
                      "--y": "25%",
                      "--r": "-3deg",
                      "--z": "4",
                    } as CSSProperties
                  }
                  aria-label="Note: Jo is sick, can't do lunch. Hand to NyxBot."
                >
                  {" "}
                  <span className="obj-in">
                    <span className="shape s-sticky">
                      <span className="o-m">Jo is sick, can't do lunch</span>
                      <span className="o-s">7:15 AM</span>
                    </span>
                    <svg
                      className="stamp"
                      viewBox="0 0 40 40"
                      aria-hidden="true"
                    >
                      <circle cx="20" cy="20" r="17" />
                      <path d="M12 20.5l5.5 5.5L28.5 14" />
                    </svg>
                  </span>
                </button>{" "}
                <button
                  className="obj"
                  type="button"
                  data-obj="call"
                  data-dest="done"
                  style={
                    {
                      "--x": "0%",
                      "--y": "58%",
                      "--r": "4deg",
                      "--z": "3",
                    } as CSSProperties
                  }
                  aria-label="Missed call from Mr Tan. Hand to NyxBot."
                >
                  {" "}
                  <span className="obj-in">
                    <span className="shape s-call">
                      <span className="o-l">Missed call</span>
                      <span className="o-m">Mr Tan</span>
                      <span className="o-s">6:05 PM, about Friday</span>
                    </span>
                    <svg
                      className="stamp"
                      viewBox="0 0 40 40"
                      aria-hidden="true"
                    >
                      <circle cx="20" cy="20" r="17" />
                      <path d="M12 20.5l5.5 5.5L28.5 14" />
                    </svg>
                  </span>
                </button>{" "}
                <button
                  className="obj"
                  type="button"
                  data-obj="cal"
                  data-dest="done"
                  style={
                    {
                      "--x": "27%",
                      "--y": "62%",
                      "--r": "6deg",
                      "--z": "5",
                    } as CSSProperties
                  }
                  aria-label="Morning brief, weekdays at 8. Hand to NyxBot."
                >
                  {" "}
                  <span className="obj-in">
                    <span className="shape s-cal">
                      <span className="cal-top">Weekdays</span>
                      <span className="cal-big">8:00</span>
                      <span className="o-s">Morning brief</span>
                    </span>
                    <svg
                      className="stamp"
                      viewBox="0 0 40 40"
                      aria-hidden="true"
                    >
                      <circle cx="20" cy="20" r="17" />
                      <path d="M12 20.5l5.5 5.5L28.5 14" />
                    </svg>
                  </span>
                </button>{" "}
              </div>{" "}
              <p className="hint">
                Drag a slip into a tray, or press Enter on it.
              </p>{" "}
            </div>{" "}
            <div className="trays" data-trays="">
              {" "}
              <div className="tray" data-tray="done">
                {" "}
                <p className="tray-h">
                  {"Done by NyxBot "}
                  <span data-h-done="">nothing yet</span>
                </p>{" "}
                <ol>
                  {" "}
                  <li className="slot" data-slot="slip">
                    <span className="slot-hole" />
                    <p className="slot-text">
                      <b>Sarah's table.</b>
                      {
                        " Booked for Sunday 7:30 PM, one vegan noted. Reply sent."
                      }
                    </p>
                    <img src="/landing/nyx/icons/telegram.svg" alt="Telegram" />
                  </li>{" "}
                  <li className="slot" data-slot="letter">
                    <span className="slot-hole" />
                    <p className="slot-text">
                      <b>A question on X.</b>
                      {" Answered from your menu notes."}
                    </p>
                    <img src="/landing/nyx/icons/x.svg" alt="X" />
                  </li>{" "}
                  <li className="slot" data-slot="receipt">
                    <span className="slot-hole" />
                    <p className="slot-text">
                      <b>Saturday's takings.</b>
                      {" Added to your sheet, row 212."}
                    </p>
                    <img
                      src="/landing/nyx/icons/google-sheets.svg"
                      alt="Google Sheets"
                    />
                  </li>{" "}
                  <li className="slot" data-slot="sticky">
                    <span className="slot-hole" />
                    <p className="slot-text">
                      <b>Jo is sick.</b>
                      {" Sam said yes to lunch. Rota updated."}
                    </p>
                    <img
                      src="/landing/nyx/icons/google-calendar.svg"
                      alt="Google Calendar"
                    />
                  </li>{" "}
                  <li className="slot" data-slot="call">
                    <span className="slot-hole" />
                    <p className="slot-text">
                      <b>Mr Tan's missed call.</b>
                      {" Called back. Friday delivery confirmed."}
                    </p>
                    <img src="/landing/nyx/icons/twilio.svg" alt="Twilio" />
                  </li>{" "}
                  <li className="slot" data-slot="cal">
                    <span className="slot-hole" />
                    <p className="slot-text">
                      <b>Your morning brief.</b>
                      {" Sent to your phone at 8:00."}
                    </p>
                    <img src="/landing/nyx/icons/telegram.svg" alt="Telegram" />
                  </li>{" "}
                  <li className="slot" data-slot="invoice-done" hidden>
                    <span className="slot-hole" />
                    <p className="slot-text">
                      <b>Harbour Fish Co.</b>
                      {" You approved. Confirmation sent at 8:01."}
                    </p>
                    <img src="/landing/nyx/icons/gmail.svg" alt="Gmail" />
                  </li>{" "}
                </ol>{" "}
              </div>{" "}
              <div className="tray tray--wait" data-tray="wait">
                {" "}
                <p className="tray-h">
                  {"Waiting for approval "}
                  <span data-h-wait="">nothing yet</span>
                </p>{" "}
                <ol>
                  {" "}
                  <li className="slot slot--wait" data-slot="invoice">
                    {" "}
                    <span className="slot-hole" />{" "}
                    <p className="slot-text" data-wait-text="">
                      <b>Harbour Fish Co. raised Friday's order by 8%.</b>
                      {" A reply is drafted. Money decisions wait for you."}
                    </p>{" "}
                    <div className="slot-act">
                      {" "}
                      <button className="b-ok" type="button" data-approve="">
                        Approve the reply
                      </button>{" "}
                      <button className="b-no" type="button" data-deny="">
                        Deny
                      </button>{" "}
                      <button
                        className="b-undo"
                        type="button"
                        data-undo=""
                        hidden
                      >
                        Undo
                      </button>{" "}
                    </div>{" "}
                  </li>{" "}
                </ol>{" "}
                <p className="tray-none" data-wait-none="" hidden>
                  Nothing is waiting for you.
                </p>{" "}
              </div>{" "}
            </div>{" "}
          </div>
        </section>{" "}
        {/* ============ 3. YOUR OWN AGENT ============ */}{" "}
        <section className="agent" id="agent">
          {" "}
          <div className="wrap agent-grid">
            {" "}
            <div>
              {" "}
              <h2>
                {"Already use Claude Code or Cursor? "}
                <em>Connect it.</em>
              </h2>{" "}
              <p className="lede">
                Point any MCP client at NyxID and it can use the same apps. Use a separate Agent Key for each agent and choose its service scope. NyxID adds the real login at the
                proxy, so the agent never holds it.
              </p>{" "}
              <div className="agent-who">
                {" "}
                <span className="chip">
                  <img src="/landing/nyx/icons/claude.svg" alt="" />
                  Claude Code
                </span>{" "}
                <span className="chip">
                  <img
                    className="ic-ink"
                    src="/landing/nyx/icons/cursor.svg"
                    alt=""
                  />
                  Cursor
                </span>{" "}
                <span className="chip">
                  <img
                    className="ic-ink"
                    src="/landing/nyx/icons/openai.svg"
                    alt=""
                  />
                  Codex
                </span>{" "}
              </div>{" "}
              <div className="term">
                {" "}
                <div
                  className="term-tabs"
                  role="tablist"
                  aria-label="Setup command"
                >
                  {" "}
                  <button
                    type="button"
                    role="tab"
                    aria-selected="true"
                    data-term="claude"
                  >
                    Claude Code
                  </button>{" "}
                  <button
                    type="button"
                    role="tab"
                    aria-selected="false"
                    data-term="cursor"
                  >
                    Cursor
                  </button>{" "}
                  <button className="term-copy" type="button" data-copy="">
                    Copy
                  </button>{" "}
                </div>{" "}
                <pre data-term-out="">
                  <s>$</s>
                  {" claude mcp add --transport http --scope user "}
                  <i>nyxid</i>
                  {" https://nyx-api.chrono-ai.fun/mcp"}
                </pre>{" "}
              </div>{" "}
              <a className="link" href={s.siteHref("/agents")}>Your agent <span aria-hidden="true">→</span></a>
            </div>{" "}
            <div className="swap">
              {" "}
              <div
                className="flow"
                data-flow=""
                data-step="4"
                role="img"
                aria-label="Claude Code sends its own NyxID key. NyxID checks it, swaps in the real Stripe key from its vault, and Stripe creates the refund. The agent never holds the real key."
              >
                {" "}
                <div className="flow-card" data-flow-at="0">
                  {" "}
                  <p className="fc-h">
                    <span className="fc-ic">
                      <img src="/landing/nyx/icons/claude.svg" alt="" />
                    </span>
                    <b>Claude Code</b>
                    <span className="fc-tag">your agent</span>
                  </p>{" "}
                  <p className="fc-line">Asks Stripe to refund $40</p>{" "}
                  <p className="fc-key">
                    {"Sends "}
                    <span className="k-agent">
                      <svg aria-hidden="true">
                        <use href="#lock" />
                      </svg>
                      nyxid_ag_7f3c
                    </span>
                    its own NyxID key
                  </p>{" "}
                </div>{" "}
                <div className="flow-wire" data-w="1">
                  <i />
                </div>{" "}
                <div className="flow-card core" data-flow-at="2">
                  {" "}
                  <p className="fc-h">
                    <span className="fc-ic">
                      <img src="/nyxid-icon-white.svg" alt="" />
                    </span>
                    <b>NyxID</b>
                    <span className="fc-tag">checks, then swaps the key</span>
                  </p>{" "}
                  <ul className="fc-checks">
                    {" "}
                    <li>
                      <i>
                        <svg>
                          <use href="#ck" />
                        </svg>
                      </i>
                      The key belongs to this agent
                    </li>{" "}
                    <li>
                      <i>
                        <svg>
                          <use href="#ck" />
                        </svg>
                      </i>
                      Stripe is in its scope
                    </li>{" "}
                    <li>
                      <i>
                        <svg>
                          <use href="#ck" />
                        </svg>
                      </i>
                      Your rule for refunds is Allow
                    </li>{" "}
                  </ul>{" "}
                  <p className="fc-swap">
                    {"Swaps "}
                    <span className="k-agent">nyxid_ag_7f3c</span>
                    <svg viewBox="0 0 16 16">
                      <path d="M3 8h10M9 4l4 4-4 4" />
                    </svg>
                    <span className="k-real">
                      <svg aria-hidden="true">
                        <use href="#lock" />
                      </svg>
                      real Stripe key
                    </span>
                    from its vault
                  </p>{" "}
                </div>{" "}
                <div className="flow-wire" data-w="3">
                  <i />
                </div>{" "}
                <div className="flow-card app" data-flow-at="4">
                  {" "}
                  <p className="fc-h">
                    <span className="fc-ic">
                      <img src="/landing/nyx/icons/stripe.svg" alt="" />
                    </span>
                    <b>Stripe</b>
                    <span className="fc-tag">the app</span>
                  </p>{" "}
                  <p className="fc-key">
                    {"Receives "}
                    <span className="k-real">
                      <svg aria-hidden="true">
                        <use href="#lock" />
                      </svg>
                      sk_live_••••4242
                    </span>
                    the real key, never seen by the agent
                  </p>{" "}
                  <p className="fc-done">
                    <i>
                      <svg>
                        <use href="#ck" />
                      </svg>
                    </i>
                    Refund of $40 created
                  </p>{" "}
                </div>{" "}
              </div>{" "}
              <div className="swap-facts">
                {" "}
                <div>
                  <b>One scoped key per agent</b>
                  Agent A reaches Slack and Gmail. Agent B reaches only your
                  internal API.
                </div>{" "}
                <div>
                  <b>Revoke one, keep the rest</b>
                  End any agent's access without touching the login behind it.
                </div>{" "}
              </div>{" "}
            </div>{" "}
          </div>{" "}
        </section>{" "}
        {/* ============ 4. APPS ============ */}{" "}
        <section className="apps" id="apps">
          {" "}
          <div className="wrap">
            {" "}
            <div className="apps-head">
              {" "}
              <h2>
                {"Ask in a chat and the work lands "}
                <em>in your apps.</em>
              </h2>{" "}
              <p className="lede">
                Connect the apps your business runs on. Give NyxBot or your own
                agent access through a connected account or API key.
              </p>{" "}
            </div>{" "}
            <div className="hub" data-hub="">
              {" "}
              <svg className="hub-lines" aria-hidden="true" />{" "}
              <div className="hub-col hub-in">
                {" "}
                <p className="hub-col-l">Ask from</p>{" "}
                <span className="tile" data-node="in">
                  <img src="/landing/nyx/icons/telegram.svg" alt="" />
                  Telegram
                </span>{" "}
                <span className="tile" data-node="in">
                  <img src="/landing/nyx/icons/x.svg" alt="" />X
                </span>{" "}
                <span className="tile" data-node="in">
                  <img src="/landing/nyx/icons/whatsapp.svg" alt="" />
                  WhatsApp <span className="tile-status">Coming soon</span>
                </span>{" "}
                <span className="tile" data-node="in">
                  <img src="/landing/nyx/icons/slack.svg" alt="" />
                  Slack
                </span>{" "}
                <span className="tile" data-node="in">
                  <img src="/landing/nyx/icons/discord.svg" alt="" />
                  Discord
                </span>{" "}
              </div>{" "}
              <div className="hub-core" data-core="">
                <img src="/nyxid-coloured-icon.svg" alt="NyxID" />
              </div>{" "}
              <div className="hub-col hub-out">
                {" "}
                <p className="hub-col-l">It works in</p>{" "}
                <span className="tile" data-node="out">
                  <img src="/landing/nyx/icons/google-calendar.svg" alt="" />
                  Google Calendar
                </span>{" "}
                <span className="tile" data-node="out">
                  <img src="/landing/nyx/icons/gmail.svg" alt="" />
                  Gmail
                </span>{" "}
                <span className="tile" data-node="out">
                  <img src="/landing/nyx/icons/google-sheets.svg" alt="" />
                  Google Sheets
                </span>{" "}
                <span className="tile" data-node="out">
                  <img src="/landing/nyx/icons/google-drive.svg" alt="" />
                  Google Drive
                </span>{" "}
                <span className="tile" data-node="out">
                  <img src="/landing/nyx/icons/notion.svg" alt="" />
                  Notion
                </span>{" "}
                <span className="tile" data-node="out">
                  <img src="/landing/nyx/icons/stripe.svg" alt="" />
                  Stripe
                </span>{" "}
                <span className="tile" data-node="out">
                  <img src="/landing/nyx/icons/twilio.svg" alt="" />
                  Twilio
                </span>{" "}
                <span className="tile" data-node="out">
                  <img src="/landing/nyx/icons/github.svg" alt="" />
                  GitHub
                </span>{" "}
              </div>{" "}
            </div>{" "}
            <ToolCatalog />
            <div className="apps-note">
              {" "}
              <p>
                <b>Your app is missing?</b>
                {
                  " Any REST API with an OpenAPI file becomes a tool NyxBot can use."
                }
              </p>{" "}
              <a className="link" href={s.siteHref("/docs/ai/guides/wrap-rest-api-as-mcp")}>
                {"API connection guide "}
                <span className="arr" aria-hidden="true">
                  →
                </span>
              </a>{" "}
            </div>{" "}
          </div>{" "}
        </section>{" "}
        {/* ============ 5. READY-MADE JOBS ============ */}{" "}
        <section className="jobs" id="jobs">
          {" "}
          <div className="wrap">
            {" "}
            <div className="jobs-head">
              {" "}
              <h2>
                {"Start with a job."}
              </h2>{" "}
              <a className="link" href={s.siteHref("/solutions")}>
                {"Solutions "}
                <span className="arr" aria-hidden="true">
                  →
                </span>
              </a>{" "}
            </div>{" "}
            <HomeJobs />
            <div className="jobs-bar" aria-hidden="true">
              <i data-jobs-bar="" />
            </div>{" "}
          </div>{" "}
        </section>{" "}
      {/* ============ 8. CLOSE ============ */}{" "}
      <section className="close" id="end" aria-labelledby="closing-title">
        {" "}
        <div className="close-art" aria-hidden="true">
          <canvas data-nyx="/landing/nyx/art/moreau-nyx.jpg" />
        </div>{" "}
        <div className="wrap close-in" data-close="">
          {" "}
          <p className="big" id="closing-title" data-split="">
            {"Give NyxBot "}
            <em>one task</em>
            {" today."}
          </p>{" "}
          <div className="close-cta">
            {" "}
            <a className="btn btn-lilac" href={s.authHref("register")}>
              {"Get started "}
              <span className="arr" aria-hidden="true">
                →
              </span>
            </a>{" "}
            <a className="link" href={s.siteHref("/pricing")}>Pricing <span className="arr" aria-hidden="true">→</span></a>
            <a className="link" href="https://github.com/ChronoAIProject/NyxID">
              {"Self-host with Docker "}
              <span className="arr" aria-hidden="true">
                →
              </span>
            </a>{" "}
          </div>{" "}
        </div>{" "}
      </section>
      </main>{" "}
      <PublicFooter siteHref={s.siteHref} localeHref={s.localeHref} />{" "}
    </div>
  );
}
