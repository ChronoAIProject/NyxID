// NyxID home page, ported from the "light night" design (mockups/landing-combined/nyxid-light-night.html).
// The markup below is the design's markup converted to JSX; nyx-home-motion.js runs its animations.
import { useLayoutEffect, useRef } from "react";
import type { CSSProperties } from "react";
import { useWebsite } from "../website-context";
import { HomeJobs } from "./nyx-home-jobs";
import { ToolCatalog } from "./nyx-home-tools";
import { PublicHeader, PublicFooter } from "../../public-site/public-chrome";
import { usePublicThemeElement } from "../../public-site/public-theme";
import { homeText } from "./home-copy";
import "./nyx-home.css";

export default function NyxHomePage() {
  const s = useWebsite();
  const t = (english: Parameters<typeof homeText>[0]) => homeText(english, s.language);
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
    <div ref={ref} className="nyx-home nyx-public" lang={s.language === "zh" ? "zh-CN" : "en"}>
      {" "}
      <div className="page-sky" aria-hidden="true" />{" "}
      <a className="skip" href="#main">
        {t("Skip to content")}
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
            <span className="o-m">{t("Table for 4 at 7:30")}</span>
            <span className="o-s">{t("Sarah, one vegan")}</span>
          </span>
        </span>{" "}
        <span data-k="cal">
          <span className="shape s-cal">
            <span className="cal-top">{t("Every weekday")}</span>
            <span className="cal-big">08:00</span>
            <span className="o-s">{t("Morning brief")}</span>
          </span>
        </span>{" "}
        <span data-k="letter">
          <span className="shape s-letter">
            <svg viewBox="0 0 100 66" preserveAspectRatio="none">
              <path d="M0 0 L50 34 L100 0" />
            </svg>
            <span className="o-l">X</span>
            <span className="o-m">{t("Open on Monday?")}</span>
            <span className="o-s">{t("A DM to answer")}</span>
          </span>
        </span>{" "}
        <span data-k="sticky">
          <span className="shape s-sticky">
            <span className="o-m">{t("Call Mr Tan back")}</span>
            <span className="o-s">{t("Missed call, 6:10 PM")}</span>
          </span>
        </span>{" "}
        <span data-k="invoice">
          <span className="shape s-invoice">
            <span className="o-l">Stripe</span>
            <span className="o-m">{t("Refund $40")}</span>
            <span className="o-s">{t("Ask first")}</span>
          </span>
        </span>{" "}
        <span data-k="slip">
          <span className="shape s-slip">
            <span className="o-l">Claude Code</span>
            <span className="o-m">{t("Open a pull request")}</span>
            <span className="o-s">GitHub</span>
          </span>
        </span>{" "}
        <span data-k="receipt">
          <span className="shape s-receipt">
            <span className="o-l">Google Sheets</span>
            <span className="row">
              <span>{t("Takings")}</span>
              <b>{t("Sat")}</b>
            </span>
            <span className="row">
              <span>{t("Row")}</span>
              <b>212</b>
            </span>
            <span className="row">
              <span>{t("Rule")}</span>
              <b>{t("Allow")}</b>
            </span>
          </span>
        </span>{" "}
      </div>{" "}
      <PublicHeader home language={s.language} siteHref={s.siteHref} authHref={s.authHref} localeHref={s.localeHref} />
      <nav className="hud" aria-label={t("Page position")} data-hud="">
        {" "}
        <a href="#top">
          <i />
          <span>{t("01 Ask")}</span>
        </a>{" "}
        <a href="#pile">
          <i />
          <span>{t("02 NyxBot")}</span>
        </a>{" "}
        <a href="#agent">
          <i />
          <span>{t("03 Your agent")}</span>
        </a>{" "}
        <a href="#apps">
          <i />
          <span>{t("04 Tools")}</span>
        </a>{" "}
        <a href="#jobs">
          <i />
          <span>{t("05 Solutions")}</span>
        </a>{" "}
        <a href="#end">
          <i />
          <span>{t("06 Start")}</span>
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
                <span>{t("The AI assistant")}</span>
                <span>
                  {t("that gets it ")}
                  <em>{t("done.")}</em>
                </span>
              </h1>{" "}
              <p
                className="lede"
                data-in=""
                style={{ "--d": "1" } as CSSProperties}
              >
                {t("Ask NyxBot in plain words on Telegram, X or the NyxID app. It books the table, replies to the customer and sends the morning brief, under rules you set.")}
              </p>{" "}
              <div
                className="hero-cta"
                data-in=""
                style={{ "--d": "2" } as CSSProperties}
              >
                {" "}
                <a className="btn" href={s.authHref("register")}>
                  {t("Get started ")}
                  <span className="arr" aria-hidden="true">
                    →
                  </span>
                </a>{" "}
                <a
                  className="link"
                  href="https://github.com/ChronoAIProject/NyxID"
                >
                  {t("Self-host with Docker ")}
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
                <span>{t("Open source under Apache-2.0")}</span>
                <span>
                  <img src="/landing/nyx/icons/telegram.svg" alt="Telegram" />
                  <img src="/landing/nyx/icons/x.svg" alt="X" />
                  <img
                    src="/landing/nyx/icons/whatsapp.svg"
                    alt={t("WhatsApp (coming soon)")}
                    title={t("WhatsApp — coming soon")}
                  />
                  <img src="/landing/nyx/icons/slack.svg" alt="Slack" />
                  <img src="/landing/nyx/icons/discord.svg" alt="Discord" />
                  {t("and the NyxID app")}
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
                {t(" of ")}
                <span data-total="">10</span>
                {t(" requests handled")}
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
                    {t("Done!")}
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
          aria-label={t("How NyxBot works through the night's pile")}
        >
          {" "}
          <div className="wrap pile-grid">
            {" "}
            <div className="pile-copy">
              {" "}
              <p className="section-label">{t("NyxBot at work")}</p>
              <h2>{t("The pile on your counter, sorted before you wake.")}</h2>{" "}
              <p className="lede">
                {t("Your agent works in the apps and chats you connect. You set the limits and review actions that need approval.")}
              </p>{" "}
              <div className="pile-detail-links">
                <a className="link" href={s.siteHref("/how-it-works")}>
                  {t("How it works")}{" "}
                  <span className="arr" aria-hidden="true">
                    →
                  </span>
                </a>
                <a className="link" href={s.siteHref("/security")}>
                  {t("Security")}{" "}
                  <span className="arr" aria-hidden="true">
                    →
                  </span>
                </a>
              </div>
              <p className="count" aria-live="polite">
                <span>
                  <b data-n-done="">0</b>
                  {t("done")}
                </span>
                <span>
                  <b data-n-wait="">0</b>
                  {t("waiting for approval")}
                </span>
              </p>{" "}
              <div
                className="pile"
                data-pile=""
                aria-label={t("Things on the counter")}
              >
                {" "}
                <p className="pile-empty">{t("The counter is clear.")}</p>{" "}
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
                  aria-label={t("Reservation from Sarah. Hand to NyxBot.")}
                >
                  {" "}
                  <span className="obj-in">
                    <span className="shape s-slip">
                      <span className="o-l">{t("Reservation")}</span>
                      <span className="o-m">{t("Table for 4")}</span>
                      <span className="o-s">{t("Sun 7:30 PM, Sarah")}</span>
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
                  aria-label={t("Question on X from maya_eats. Hand to NyxBot.")}
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
                      <span className="o-m">{t("Gluten-free dishes?")}</span>
                      <span className="o-s">{t("Open on Monday?")}</span>
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
                  aria-label={t("Invoice from Harbour Fish Co. Hand to NyxBot.")}
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
                      <span className="o-m">{t("Friday order")}</span>
                      <span className="o-s">{t("Price up 8%. Confirm?")}</span>
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
                  aria-label={t("Till receipt for Saturday. Hand to NyxBot.")}
                >
                  {" "}
                  <span className="obj-in">
                    <span className="shape s-receipt">
                      <span className="o-l">{t("Saturday till")}</span>
                      <span className="row">
                        <span>{t("Tables")}</span>
                        <b>38</b>
                      </span>
                      <span className="row">
                        <span>{t("Card")}</span>
                        <b>31</b>
                      </span>
                      <span className="row">
                        <span>{t("Cash")}</span>
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
                  aria-label={t("Note: Jo is sick, can't do lunch. Hand to NyxBot.")}
                >
                  {" "}
                  <span className="obj-in">
                    <span className="shape s-sticky">
                      <span className="o-m">{t("Jo is sick, can't do lunch")}</span>
                      <span className="o-s">{t("7:15 AM")}</span>
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
                  aria-label={t("Missed call from Mr Tan. Hand to NyxBot.")}
                >
                  {" "}
                  <span className="obj-in">
                    <span className="shape s-call">
                      <span className="o-l">{t("Missed call")}</span>
                      <span className="o-m">{t("Mr Tan")}</span>
                      <span className="o-s">{t("6:05 PM, about Friday")}</span>
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
                  aria-label={t("Morning brief, weekdays at 8. Hand to NyxBot.")}
                >
                  {" "}
                  <span className="obj-in">
                    <span className="shape s-cal">
                      <span className="cal-top">{t("Weekdays")}</span>
                      <span className="cal-big">8:00</span>
                      <span className="o-s">{t("Morning brief")}</span>
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
                {t("Drag a slip into a tray, or press Enter on it.")}
              </p>{" "}
            </div>{" "}
            <div className="trays" data-trays="">
              {" "}
              <div className="tray" data-tray="done">
                {" "}
                <p className="tray-h">
                  {t("Done by NyxBot ")}
                  <span data-h-done="">{t("nothing yet")}</span>
                </p>{" "}
                <ol>
                  {" "}
                  <li className="slot" data-slot="slip">
                    <span className="slot-hole" />
                    <p className="slot-text">
                      <b>{t("Sarah's table.")}</b>
                      {
                        t(" Booked for Sunday 7:30 PM, one vegan noted. Reply sent.")
                      }
                    </p>
                    <img src="/landing/nyx/icons/telegram.svg" alt="Telegram" />
                  </li>{" "}
                  <li className="slot" data-slot="letter">
                    <span className="slot-hole" />
                    <p className="slot-text">
                      <b>{t("A question on X.")}</b>
                      {t(" Answered from your menu notes.")}
                    </p>
                    <img src="/landing/nyx/icons/x.svg" alt="X" />
                  </li>{" "}
                  <li className="slot" data-slot="receipt">
                    <span className="slot-hole" />
                    <p className="slot-text">
                      <b>{t("Saturday's takings.")}</b>
                      {t(" Added to your sheet, row 212.")}
                    </p>
                    <img
                      src="/landing/nyx/icons/google-sheets.svg"
                      alt="Google Sheets"
                    />
                  </li>{" "}
                  <li className="slot" data-slot="sticky">
                    <span className="slot-hole" />
                    <p className="slot-text">
                      <b>{t("Jo is sick.")}</b>
                      {t(" Sam said yes to lunch. Rota updated.")}
                    </p>
                    <img
                      src="/landing/nyx/icons/google-calendar.svg"
                      alt="Google Calendar"
                    />
                  </li>{" "}
                  <li className="slot" data-slot="call">
                    <span className="slot-hole" />
                    <p className="slot-text">
                      <b>{t("Mr Tan's missed call.")}</b>
                      {t(" Called back. Friday delivery confirmed.")}
                    </p>
                    <img src="/landing/nyx/icons/twilio.svg" alt="Twilio" />
                  </li>{" "}
                  <li className="slot" data-slot="cal">
                    <span className="slot-hole" />
                    <p className="slot-text">
                      <b>{t("Your morning brief.")}</b>
                      {t(" Sent to your phone at 8:00.")}
                    </p>
                    <img src="/landing/nyx/icons/telegram.svg" alt="Telegram" />
                  </li>{" "}
                  <li className="slot" data-slot="invoice-done" hidden>
                    <span className="slot-hole" />
                    <p className="slot-text">
                      <b>Harbour Fish Co.</b>
                      {t(" You approved. Confirmation sent at 8:01.")}
                    </p>
                    <img src="/landing/nyx/icons/gmail.svg" alt="Gmail" />
                  </li>{" "}
                </ol>{" "}
              </div>{" "}
              <div className="tray tray--wait" data-tray="wait">
                {" "}
                <p className="tray-h">
                  {t("Waiting for approval ")}
                  <span data-h-wait="">{t("nothing yet")}</span>
                </p>{" "}
                <ol>
                  {" "}
                  <li className="slot slot--wait" data-slot="invoice">
                    {" "}
                    <span className="slot-hole" />{" "}
                    <p className="slot-text" data-wait-text="">
                      <b>{t("Harbour Fish Co. raised Friday's order by 8%.")}</b>
                      {t(" A reply is drafted. Money decisions wait for you.")}
                    </p>{" "}
                    <div className="slot-act">
                      {" "}
                      <button className="b-ok" type="button" data-approve="">
                        {t("Approve the reply")}
                      </button>{" "}
                      <button className="b-no" type="button" data-deny="">
                        {t("Deny")}
                      </button>{" "}
                      <button
                        className="b-undo"
                        type="button"
                        data-undo=""
                        hidden
                      >
                        {t("Undo")}
                      </button>{" "}
                    </div>{" "}
                  </li>{" "}
                </ol>{" "}
                <p className="tray-none" data-wait-none="" hidden>
                  {t("Nothing is waiting for you.")}
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
                {t("Already use Claude Code or Cursor? ")}
                <em>{t("Connect it.")}</em>
              </h2>{" "}
              <p className="lede">
                {t("Point any MCP client at NyxID and it can use the same apps. Use a separate Agent Key for each agent and choose its service scope. NyxID adds the real login at the proxy, so the agent never holds it.")}
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
                  aria-label={t("Setup command")}
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
                    {t("Copy")}
                  </button>{" "}
                </div>{" "}
                <pre data-term-out="">
                  <s>$</s>
                  {" claude mcp add --transport http --scope user "}
                  <i>nyxid</i>
                  {" https://nyx-api.chrono-ai.fun/mcp"}
                </pre>{" "}
              </div>{" "}
              <a className="link" href={s.siteHref("/agents")}>{t("Your agent")}<span aria-hidden="true">→</span></a>
            </div>{" "}
            <div className="swap">
              {" "}
              <div
                className="flow"
                data-flow=""
                data-step="4"
                role="img"
                aria-label={t("Claude Code sends its own NyxID key. NyxID checks it, swaps in the real Stripe key from its vault, and Stripe creates the refund. The agent never holds the real key.")}
              >
                {" "}
                <div className="flow-card" data-flow-at="0">
                  {" "}
                  <p className="fc-h">
                    <span className="fc-ic">
                      <img src="/landing/nyx/icons/claude.svg" alt="" />
                    </span>
                    <b>Claude Code</b>
                    <span className="fc-tag">{t("your agent")}</span>
                  </p>{" "}
                  <p className="fc-line">{t("Asks Stripe to refund $40")}</p>{" "}
                  <p className="fc-key">
                    {t("Sends ")}
                    <span className="k-agent">
                      <svg aria-hidden="true">
                        <use href="#lock" />
                      </svg>
                      nyxid_ag_7f3c
                    </span>
                    {t("its own NyxID key")}
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
                    <span className="fc-tag">{t("checks, then swaps the key")}</span>
                  </p>{" "}
                  <ul className="fc-checks">
                    {" "}
                    <li>
                      <i>
                        <svg>
                          <use href="#ck" />
                        </svg>
                      </i>
                      {t("The key belongs to this agent")}
                    </li>{" "}
                    <li>
                      <i>
                        <svg>
                          <use href="#ck" />
                        </svg>
                      </i>
                      {t("Stripe is in its scope")}
                    </li>{" "}
                    <li>
                      <i>
                        <svg>
                          <use href="#ck" />
                        </svg>
                      </i>
                      {t("Your rule for refunds is Allow")}
                    </li>{" "}
                  </ul>{" "}
                  <p className="fc-swap">
                    {t("Swaps ")}
                    <span className="k-agent">nyxid_ag_7f3c</span>
                    <svg viewBox="0 0 16 16">
                      <path d="M3 8h10M9 4l4 4-4 4" />
                    </svg>
                    <span className="k-real">
                      <svg aria-hidden="true">
                        <use href="#lock" />
                      </svg>
                      {t("real Stripe key")}
                    </span>
                    {t("from its vault")}
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
                    <span className="fc-tag">{t("the app")}</span>
                  </p>{" "}
                  <p className="fc-key">
                    {t("Receives ")}
                    <span className="k-real">
                      <svg aria-hidden="true">
                        <use href="#lock" />
                      </svg>
                      sk_live_••••4242
                    </span>
                    {t("the real key, never seen by the agent")}
                  </p>{" "}
                  <p className="fc-done">
                    <i>
                      <svg>
                        <use href="#ck" />
                      </svg>
                    </i>
                    {t("Refund of $40 created")}
                  </p>{" "}
                </div>{" "}
              </div>{" "}
              <div className="swap-facts">
                {" "}
                <div>
                  <b>{t("One scoped key per agent")}</b>
                  {t("Agent A reaches Slack and Gmail. Agent B reaches only your internal API.")}
                </div>{" "}
                <div>
                  <b>{t("Revoke one, keep the rest")}</b>
                  {t("End any agent's access without touching the login behind it.")}
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
                {t("Ask in a chat and the work lands ")}
                <em>{t("in your apps.")}</em>
              </h2>{" "}
              <p className="lede">
                {t("Connect the apps your business runs on. Give NyxBot or your own agent access through a connected account or API key.")}
              </p>{" "}
            </div>{" "}
            <div className="hub" data-hub="">
              {" "}
              <svg className="hub-lines" aria-hidden="true" />{" "}
              <div className="hub-col hub-in">
                {" "}
                <p className="hub-col-l">{t("Ask from")}</p>{" "}
                <span className="tile" data-node="in">
                  <img src="/landing/nyx/icons/telegram.svg" alt="" />
                  Telegram
                </span>{" "}
                <span className="tile" data-node="in">
                  <img src="/landing/nyx/icons/x.svg" alt="" />X
                </span>{" "}
                <span className="tile" data-node="in">
                  <img src="/landing/nyx/icons/whatsapp.svg" alt="" />
                  WhatsApp <span className="tile-status">{t("Coming soon")}</span>
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
                <p className="hub-col-l">{t("It works in")}</p>{" "}
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
                <b>{t("Your app is missing?")}</b>
                {
                  t(" Any REST API with an OpenAPI file becomes a tool NyxBot can use.")
                }
              </p>{" "}
              <a className="link" href={s.siteHref("/docs/ai/guides/wrap-rest-api-as-mcp")}>
                {t("API connection guide ")}
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
                {t("Start with a job.")}
              </h2>{" "}
              <a className="link" href={s.siteHref("/solutions")}>
                {t("Solutions ")}
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
            {t("Give NyxBot ")}
            <em>{t("one task")}</em>
            {t(" today.")}
          </p>{" "}
          <div className="close-cta">
            {" "}
            <a className="btn btn-lilac" href={s.authHref("register")}>
              {t("Get started ")}
              <span className="arr" aria-hidden="true">
                →
              </span>
            </a>{" "}
            <a className="link" href={s.siteHref("/pricing")}>{t("Pricing")}<span className="arr" aria-hidden="true">→</span></a>
            <a className="link" href="https://github.com/ChronoAIProject/NyxID">
              {t("Self-host with Docker ")}
              <span className="arr" aria-hidden="true">
                →
              </span>
            </a>{" "}
          </div>{" "}
        </div>{" "}
      </section>
      </main>{" "}
      <PublicFooter language={s.language} siteHref={s.siteHref} localeHref={s.localeHref} />{" "}
    </div>
  );
}
