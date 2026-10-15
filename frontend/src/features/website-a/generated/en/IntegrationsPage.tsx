// Generated from approved A, SHA-256 5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33.
// Run: node scripts/website-a/generate.mjs <baseline.html>
import { useWebsite } from "../../website-context";
export default function IntegrationsPage() {
  const s = useWebsite();
  void s;
  return (
    <>
      {"\n"}
      <section
        className="hero glowbg"
        style={{ paddingBottom: "36px", position: "relative" }}
      >
        {"\n  "}
        <div className="wrap">
          <div className="over v">{"Integrations"}</div>
          <h1
            className="h1"
            style={{ marginTop: "14px", fontSize: "var(--fs-h1)" }}
          >
            {"Connect the tools"}
            <br />
            <span className="grad">{"you already use."}</span>
          </h1>
          <p className="lead">
            {
              "145 apps and APIs across 30 categories. Connect each one with a secure sign-in link, then decide exactly what your AI may do with it."
            }
          </p>
        </div>
        {"\n"}
      </section>
      {"\n"}
      <section className="s" style={{ paddingTop: "28px" }}>
        <div className="wrap">
          {"\n  "}
          <div className="int-bar">
            {"\n    "}
            <input
              id="intQ"
              className="c-input"
              type="search"
              placeholder="Search integrations"
              aria-label="Search integrations"
              value={s.query}
              onChange={(e) => s.setQuery(e.target.value)}
            />
            {"\n    "}
            <select
              id="intCat"
              className="c-input"
              aria-label="Category"
              value={s.category}
              onChange={(e) => s.setCategory(e.target.value)}
            >
              <option value="">{"All categories"}</option>
              <option value="0">{"Docs, Notes & Knowledge"}</option>
              <option value="1">{"Messaging & Telephony"}</option>
              <option value="2">{"Developer Tools & Code"}</option>
              <option value="3">{"Spreadsheets & Databases"}</option>
              <option value="4">{"Email & Contacts"}</option>
              <option value="5">{"Calendar & Scheduling"}</option>
              <option value="6">{"Files & Storage"}</option>
              <option value="7">{"Project & Task Management"}</option>
              <option value="8">{"CRM & Sales"}</option>
              <option value="9">{"Video & Meetings"}</option>
              <option value="10">{"Customer Support"}</option>
              <option value="11">{"Payments & Banking"}</option>
              <option value="12">{"Design & Whiteboards"}</option>
              <option value="13">{"E-commerce"}</option>
              <option value="14">{"Accounting & ERP"}</option>
              <option value="15">{"Media, Music & Video"}</option>
              <option value="16">{"Marketing & Email Marketing"}</option>
              <option value="17">{"Social Media"}</option>
              <option value="18">{"Forms & Surveys"}</option>
              <option value="19">{"Monitoring & Observability"}</option>
              <option value="20">{"Analytics & BI"}</option>
              <option value="21">{"Cloud & Infrastructure"}</option>
              <option value="22">{"Websites & CMS"}</option>
              <option value="23">{"HR, Payroll & Recruiting"}</option>
              <option value="24">{"AI & Agents"}</option>
              <option value="25">{"E-signature & Legal"}</option>
              <option value="26">{"Advertising"}</option>
              <option value="27">{"Events"}</option>
              <option value="28">{"Security & Identity"}</option>
              <option value="29">{"Cloud cost"}</option>
            </select>
            {"\n    "}
            <div className="pills" id="intAv">
              <button
                className={s.filterClass("intAv", "")}
                data-v=""
                type="button"
                onClick={() => s.setFilter("intAv", "")}
                aria-pressed={s.isFilterSelected("intAv", "")}
              >
                {"All"}
              </button>
              <button
                className={s.filterClass("intAv", "1")}
                data-v="1"
                type="button"
                onClick={() => s.setFilter("intAv", "1")}
                aria-pressed={s.isFilterSelected("intAv", "1")}
              >
                {"Live"}
              </button>
              <button
                className={s.filterClass("intAv", "0")}
                data-v="0"
                type="button"
                onClick={() => s.setFilter("intAv", "0")}
                aria-pressed={s.isFilterSelected("intAv", "0")}
              >
                {"Coming soon"}
              </button>
            </div>
            {"\n  "}
          </div>
          {"\n  "}
          <p className="small" role="status">
            {s.integrationCountLabel}
          </p>
          {"\n  "}
          <div id="intGrid">
            <div
              className="int-group"
              data-group="1"
              hidden={s.integrationGroupCount("1") === 0}
            >
              <div className="int-gh">
                <h2 className="h3" style={{ fontSize: "22px" }}>
                  {"Live "}
                  <span className="small">{s.integrationGroupCount("1")}</span>
                </h2>
                <p className="small">{"Connect these today."}</p>
              </div>
              <div className="int-grid">
                <div
                  className="int big"
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
                    <small>{"Docs, Notes & Knowledge"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
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
                    <small>{"Messaging & Telephony"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
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
                    <small>{"Developer Tools & Code"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
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
                    <small>{"Developer Tools & Code"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
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
                    <small>{"Spreadsheets & Databases"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
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
                    <small>{"Email & Contacts"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
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
                    <small>{"Calendar & Scheduling"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
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
                    <small>{"Files & Storage"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
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
                    <small>{"Project & Task Management"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
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
                    <small>{"Files & Storage"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
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
                    <small>{"Developer Tools & Code"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
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
                    <small>{"Video & Meetings"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
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
                    <small>{"Files & Storage"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
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
                    <small>{"CRM & Sales"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
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
                    <small>{"Project & Task Management"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
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
                    <small>{"Docs, Notes & Knowledge"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
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
                    <small>{"Spreadsheets & Databases"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
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
                    <small>{"Email & Contacts"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
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
                    <small>{"Payments & Banking"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
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
                    <small>{"Files & Storage"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
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
                    <small>{"Calendar & Scheduling"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
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
                    <small>{"Design & Whiteboards"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
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
                    <small>{"Messaging & Telephony"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
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
                    <small>{"Customer Support"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
                  data-name="google slides"
                  data-cat="0"
                  data-av="1"
                  hidden={!s.integrationVisible("google slides", "0", "1")}
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
                    <b>{"Google Slides"}</b>
                    <small>{"Docs, Notes & Knowledge"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
                  data-name="supabase"
                  data-cat="3"
                  data-av="1"
                  hidden={!s.integrationVisible("supabase", "3", "1")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=supabase.io&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Supabase"}</b>
                    <small>{"Spreadsheets & Databases"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
                  data-name="todoist"
                  data-cat="7"
                  data-av="1"
                  hidden={!s.integrationVisible("todoist", "7", "1")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=todoist.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Todoist"}</b>
                    <small>{"Project & Task Management"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
                  data-name="gitlab"
                  data-cat="2"
                  data-av="1"
                  hidden={!s.integrationVisible("gitlab", "2", "1")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=about.gitlab.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"GitLab"}</b>
                    <small>{"Developer Tools & Code"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
                  data-name="microsoft sharepoint"
                  data-cat="6"
                  data-av="1"
                  hidden={
                    !s.integrationVisible("microsoft sharepoint", "6", "1")
                  }
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=microsoft.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Microsoft SharePoint"}</b>
                    <small>{"Files & Storage"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
                  data-name="youtube"
                  data-cat="15"
                  data-av="1"
                  hidden={!s.integrationVisible("youtube", "15", "1")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=developers.google.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"YouTube"}</b>
                    <small>{"Media, Music & Video"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
                  data-name="microsoft excel"
                  data-cat="3"
                  data-av="1"
                  hidden={!s.integrationVisible("microsoft excel", "3", "1")}
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
                    <b>{"Microsoft Excel"}</b>
                    <small>{"Spreadsheets & Databases"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
                  data-name="reddit"
                  data-cat="17"
                  data-av="1"
                  hidden={!s.integrationVisible("reddit", "17", "1")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=reddit.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Reddit"}</b>
                    <small>{"Social Media"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
                  data-name="bitbucket"
                  data-cat="2"
                  data-av="1"
                  hidden={!s.integrationVisible("bitbucket", "2", "1")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=bitbucket.org&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Bitbucket"}</b>
                    <small>{"Developer Tools & Code"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
                  data-name="linkedin"
                  data-cat="17"
                  data-av="1"
                  hidden={!s.integrationVisible("linkedin", "17", "1")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=linkedin.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"LinkedIn"}</b>
                    <small>{"Social Media"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
                  data-name="sentry"
                  data-cat="2"
                  data-av="1"
                  hidden={!s.integrationVisible("sentry", "2", "1")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=sentry.io&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Sentry"}</b>
                    <small>{"Developer Tools & Code"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
                  data-name="pagerduty"
                  data-cat="19"
                  data-av="1"
                  hidden={!s.integrationVisible("pagerduty", "19", "1")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=pagerduty.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"PagerDuty"}</b>
                    <small>{"Monitoring & Observability"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
                  data-name="facebook pages"
                  data-cat="17"
                  data-av="1"
                  hidden={!s.integrationVisible("facebook pages", "17", "1")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=developers.facebook.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Facebook Pages"}</b>
                    <small>{"Social Media"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
                  data-name="posthog"
                  data-cat="20"
                  data-av="1"
                  hidden={!s.integrationVisible("posthog", "20", "1")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=posthog.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"PostHog"}</b>
                    <small>{"Analytics & BI"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
                  data-name="square"
                  data-cat="11"
                  data-av="1"
                  hidden={!s.integrationVisible("square", "11", "1")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=squareup.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Square"}</b>
                    <small>{"Payments & Banking"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
                  data-name="dialpad"
                  data-cat="1"
                  data-av="1"
                  hidden={!s.integrationVisible("dialpad", "1", "1")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=dialpad.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Dialpad"}</b>
                    <small>{"Messaging & Telephony"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
                  data-name="x (twitter)"
                  data-cat="17"
                  data-av="1"
                  hidden={!s.integrationVisible("x (twitter)", "17", "1")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=x.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"X (Twitter)"}</b>
                    <small>{"Social Media"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
                  data-name="attio"
                  data-cat="8"
                  data-av="1"
                  hidden={!s.integrationVisible("attio", "8", "1")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=attio.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Attio"}</b>
                    <small>{"CRM & Sales"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
                  data-name="productboard"
                  data-cat="7"
                  data-av="1"
                  hidden={!s.integrationVisible("productboard", "7", "1")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=productboard.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Productboard"}</b>
                    <small>{"Project & Task Management"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
                  data-name="railway"
                  data-cat="2"
                  data-av="1"
                  hidden={!s.integrationVisible("railway", "2", "1")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=railway.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Railway"}</b>
                    <small>{"Developer Tools & Code"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
                  data-name="cloudflare"
                  data-cat="21"
                  data-av="1"
                  hidden={!s.integrationVisible("cloudflare", "21", "1")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=cloudflare.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Cloudflare"}</b>
                    <small>{"Cloud & Infrastructure"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
                  data-name="microsoft onenote"
                  data-cat="0"
                  data-av="1"
                  hidden={!s.integrationVisible("microsoft onenote", "0", "1")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=microsoft.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Microsoft OneNote"}</b>
                    <small>{"Docs, Notes & Knowledge"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
                  data-name="discord"
                  data-cat="1"
                  data-av="1"
                  hidden={!s.integrationVisible("discord", "1", "1")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=discord.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Discord"}</b>
                    <small>{"Messaging & Telephony"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
                  data-name="google bigquery"
                  data-cat="3"
                  data-av="1"
                  hidden={!s.integrationVisible("google bigquery", "3", "1")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=cloud.google.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Google BigQuery"}</b>
                    <small>{"Spreadsheets & Databases"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
                  data-name="microsoft to do"
                  data-cat="7"
                  data-av="1"
                  hidden={!s.integrationVisible("microsoft to do", "7", "1")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=todo.microsoft.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Microsoft To Do"}</b>
                    <small>{"Project & Task Management"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
                  data-name="google cloud pub/sub"
                  data-cat="21"
                  data-av="1"
                  hidden={
                    !s.integrationVisible("google cloud pub/sub", "21", "1")
                  }
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=cloud.google.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Google Cloud Pub/Sub"}</b>
                    <small>{"Cloud & Infrastructure"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
                  data-name="miro"
                  data-cat="12"
                  data-av="1"
                  hidden={!s.integrationVisible("miro", "12", "1")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=miro.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Miro"}</b>
                    <small>{"Design & Whiteboards"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
                  data-name="instagram"
                  data-cat="17"
                  data-av="1"
                  hidden={!s.integrationVisible("instagram", "17", "1")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=instagram.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Instagram"}</b>
                    <small>{"Social Media"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
                  data-name="eventbrite"
                  data-cat="27"
                  data-av="1"
                  hidden={!s.integrationVisible("eventbrite", "27", "1")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=eventbrite.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Eventbrite"}</b>
                    <small>{"Events"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
                  data-name="capsule crm"
                  data-cat="8"
                  data-av="1"
                  hidden={!s.integrationVisible("capsule crm", "8", "1")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=capsulecrm.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Capsule CRM"}</b>
                    <small>{"CRM & Sales"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
                  data-name="crowdin"
                  data-cat="2"
                  data-av="1"
                  hidden={!s.integrationVisible("crowdin", "2", "1")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=crowdin.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Crowdin"}</b>
                    <small>{"Developer Tools & Code"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
                <div
                  className="int big"
                  data-name="google cloud billing budgets"
                  data-cat="29"
                  data-av="1"
                  hidden={
                    !s.integrationVisible(
                      "google cloud billing budgets",
                      "29",
                      "1",
                    )
                  }
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=cloud.google.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Google Cloud Billing Budgets"}</b>
                    <small>{"Cloud cost"}</small>
                  </span>
                  <span className="badge b-ok">{"● Live"}</span>
                </div>
              </div>
            </div>
            <div
              className="int-group"
              data-group="0"
              hidden={s.integrationGroupCount("0") === 0}
            >
              <div className="int-gh">
                <h2 className="h3" style={{ fontSize: "22px" }}>
                  {"Coming soon "}
                  <span className="small">{s.integrationGroupCount("0")}</span>
                </h2>
                <p className="small">
                  {"On our roadmap. Tell us which you need first."}
                </p>
              </div>
              <div className="int-grid">
                <div
                  className="int big"
                  data-name="salesforce"
                  data-cat="8"
                  data-av="0"
                  hidden={!s.integrationVisible("salesforce", "8", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=salesforce.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Salesforce"}</b>
                    <small>{"CRM & Sales"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="zendesk"
                  data-cat="10"
                  data-av="0"
                  hidden={!s.integrationVisible("zendesk", "10", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=zendesk.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Zendesk"}</b>
                    <small>{"Customer Support"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="trello"
                  data-cat="7"
                  data-av="0"
                  hidden={!s.integrationVisible("trello", "7", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=trello.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Trello"}</b>
                    <small>{"Project & Task Management"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="confluence"
                  data-cat="0"
                  data-av="0"
                  hidden={!s.integrationVisible("confluence", "0", "0")}
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
                    <b>{"Confluence"}</b>
                    <small>{"Docs, Notes & Knowledge"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="monday.com"
                  data-cat="7"
                  data-av="0"
                  hidden={!s.integrationVisible("monday.com", "7", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=monday.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"monday.com"}</b>
                    <small>{"Project & Task Management"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="shopify"
                  data-cat="13"
                  data-av="0"
                  hidden={!s.integrationVisible("shopify", "13", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=partner.shopify.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Shopify"}</b>
                    <small>{"E-commerce"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="quickbooks online"
                  data-cat="14"
                  data-av="0"
                  hidden={!s.integrationVisible("quickbooks online", "14", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=quickbooks.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"QuickBooks Online"}</b>
                    <small>{"Accounting & ERP"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="mailchimp"
                  data-cat="16"
                  data-av="0"
                  hidden={!s.integrationVisible("mailchimp", "16", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=mailchimp.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Mailchimp"}</b>
                    <small>{"Marketing & Email Marketing"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="canva"
                  data-cat="12"
                  data-av="0"
                  hidden={!s.integrationVisible("canva", "12", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=canva.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Canva"}</b>
                    <small>{"Design & Whiteboards"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="typeform"
                  data-cat="18"
                  data-av="0"
                  hidden={!s.integrationVisible("typeform", "18", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=typeform.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Typeform"}</b>
                    <small>{"Forms & Surveys"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="google tasks"
                  data-cat="7"
                  data-av="0"
                  hidden={!s.integrationVisible("google tasks", "7", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=developers.google.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Google Tasks"}</b>
                    <small>{"Project & Task Management"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="harvest"
                  data-cat="7"
                  data-av="0"
                  hidden={!s.integrationVisible("harvest", "7", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=getharvest.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Harvest"}</b>
                    <small>{"Project & Task Management"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="vercel"
                  data-cat="2"
                  data-av="0"
                  hidden={!s.integrationVisible("vercel", "2", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=vercel.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Vercel"}</b>
                    <small>{"Developer Tools & Code"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="wrike"
                  data-cat="7"
                  data-av="0"
                  hidden={!s.integrationVisible("wrike", "7", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=wrike.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Wrike"}</b>
                    <small>{"Project & Task Management"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="pipedrive"
                  data-cat="8"
                  data-av="0"
                  hidden={!s.integrationVisible("pipedrive", "8", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=pipedrive.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Pipedrive"}</b>
                    <small>{"CRM & Sales"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="snowflake"
                  data-cat="3"
                  data-av="0"
                  hidden={!s.integrationVisible("snowflake", "3", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=snowflake.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Snowflake"}</b>
                    <small>{"Spreadsheets & Databases"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="azure devops"
                  data-cat="2"
                  data-av="0"
                  hidden={!s.integrationVisible("azure devops", "2", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=azure.microsoft.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Azure DevOps"}</b>
                    <small>{"Developer Tools & Code"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="basecamp"
                  data-cat="7"
                  data-av="0"
                  hidden={!s.integrationVisible("basecamp", "7", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=basecamp.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Basecamp"}</b>
                    <small>{"Project & Task Management"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="pinterest"
                  data-cat="17"
                  data-av="0"
                  hidden={!s.integrationVisible("pinterest", "17", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=pinterest.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Pinterest"}</b>
                    <small>{"Social Media"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="netlify"
                  data-cat="2"
                  data-av="0"
                  hidden={!s.integrationVisible("netlify", "2", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=netlify.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Netlify"}</b>
                    <small>{"Developer Tools & Code"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="zoho mail"
                  data-cat="4"
                  data-av="0"
                  hidden={!s.integrationVisible("zoho mail", "4", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=mail.zoho.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Zoho Mail"}</b>
                    <small>{"Email & Contacts"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="google meet"
                  data-cat="9"
                  data-av="0"
                  hidden={!s.integrationVisible("google meet", "9", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=meet.google.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Google Meet"}</b>
                    <small>{"Video & Meetings"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="webex"
                  data-cat="9"
                  data-av="0"
                  hidden={!s.integrationVisible("webex", "9", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=webex.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Webex"}</b>
                    <small>{"Video & Meetings"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="fathom"
                  data-cat="9"
                  data-av="0"
                  hidden={!s.integrationVisible("fathom", "9", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=fathom.ai&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Fathom"}</b>
                    <small>{"Video & Meetings"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="servicenow"
                  data-cat="21"
                  data-av="0"
                  hidden={!s.integrationVisible("servicenow", "21", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=servicenow.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"ServiceNow"}</b>
                    <small>{"Cloud & Infrastructure"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="google analytics"
                  data-cat="20"
                  data-av="0"
                  hidden={!s.integrationVisible("google analytics", "20", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=analytics.google.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Google Analytics"}</b>
                    <small>{"Analytics & BI"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="webflow"
                  data-cat="22"
                  data-av="0"
                  hidden={!s.integrationVisible("webflow", "22", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=webflow.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Webflow"}</b>
                    <small>{"Websites & CMS"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="ticktick"
                  data-cat="7"
                  data-av="0"
                  hidden={!s.integrationVisible("ticktick", "7", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=ticktick.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"TickTick"}</b>
                    <small>{"Project & Task Management"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="cal.com"
                  data-cat="5"
                  data-av="0"
                  hidden={!s.integrationVisible("cal.com", "5", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=cal.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Cal.com"}</b>
                    <small>{"Calendar & Scheduling"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="front"
                  data-cat="10"
                  data-av="0"
                  hidden={!s.integrationVisible("front", "10", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=frontapp.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Front"}</b>
                    <small>{"Customer Support"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="whatsapp business"
                  data-cat="1"
                  data-av="0"
                  hidden={!s.integrationVisible("whatsapp business", "1", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=business.whatsapp.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"WhatsApp Business"}</b>
                    <small>{"Messaging & Telephony"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="gong"
                  data-cat="9"
                  data-av="0"
                  hidden={!s.integrationVisible("gong", "9", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=gong.io&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Gong"}</b>
                    <small>{"Video & Meetings"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="google contacts"
                  data-cat="4"
                  data-av="0"
                  hidden={!s.integrationVisible("google contacts", "4", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=contacts.google.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Google Contacts"}</b>
                    <small>{"Email & Contacts"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="klaviyo"
                  data-cat="16"
                  data-av="0"
                  hidden={!s.integrationVisible("klaviyo", "16", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=klaviyo.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Klaviyo"}</b>
                    <small>{"Marketing & Email Marketing"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="microsoft dynamics 365"
                  data-cat="8"
                  data-av="0"
                  hidden={
                    !s.integrationVisible("microsoft dynamics 365", "8", "0")
                  }
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=microsoft.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Microsoft Dynamics 365"}</b>
                    <small>{"CRM & Sales"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="egnyte"
                  data-cat="6"
                  data-av="0"
                  hidden={!s.integrationVisible("egnyte", "6", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=egnyte.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Egnyte"}</b>
                    <small>{"Files & Storage"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="gorgias"
                  data-cat="10"
                  data-av="0"
                  hidden={!s.integrationVisible("gorgias", "10", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=gorgias.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Gorgias"}</b>
                    <small>{"Customer Support"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="close"
                  data-cat="8"
                  data-av="0"
                  hidden={!s.integrationVisible("close", "8", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=close.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Close"}</b>
                    <small>{"CRM & Sales"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="smartsheet"
                  data-cat="3"
                  data-av="0"
                  hidden={!s.integrationVisible("smartsheet", "3", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=smartsheet.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Smartsheet"}</b>
                    <small>{"Spreadsheets & Databases"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="zoho books"
                  data-cat="14"
                  data-av="0"
                  hidden={!s.integrationVisible("zoho books", "14", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=books.zoho.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Zoho Books"}</b>
                    <small>{"Accounting & ERP"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="help scout"
                  data-cat="10"
                  data-av="0"
                  hidden={!s.integrationVisible("help scout", "10", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=helpscout.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Help Scout"}</b>
                    <small>{"Customer Support"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="zoho desk"
                  data-cat="10"
                  data-av="0"
                  hidden={!s.integrationVisible("zoho desk", "10", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=zoho.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Zoho Desk"}</b>
                    <small>{"Customer Support"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="xero"
                  data-cat="14"
                  data-av="0"
                  hidden={!s.integrationVisible("xero", "14", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=xero.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Xero"}</b>
                    <small>{"Accounting & ERP"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="google search console"
                  data-cat="20"
                  data-av="0"
                  hidden={
                    !s.integrationVisible("google search console", "20", "0")
                  }
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=search.google.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Google Search Console"}</b>
                    <small>{"Analytics & BI"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="apollo"
                  data-cat="8"
                  data-av="0"
                  hidden={!s.integrationVisible("apollo", "8", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=apollo.io&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Apollo"}</b>
                    <small>{"CRM & Sales"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="launchdarkly"
                  data-cat="2"
                  data-av="0"
                  hidden={!s.integrationVisible("launchdarkly", "2", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=launchdarkly.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"LaunchDarkly"}</b>
                    <small>{"Developer Tools & Code"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="mural"
                  data-cat="12"
                  data-av="0"
                  hidden={!s.integrationVisible("mural", "12", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=mural.co&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Mural"}</b>
                    <small>{"Design & Whiteboards"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="paypal"
                  data-cat="11"
                  data-av="0"
                  hidden={!s.integrationVisible("paypal", "11", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=paypal.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"PayPal"}</b>
                    <small>{"Payments & Banking"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="greenhouse"
                  data-cat="23"
                  data-av="0"
                  hidden={!s.integrationVisible("greenhouse", "23", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=greenhouse.io&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Greenhouse"}</b>
                    <small>{"HR, Payroll & Recruiting"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="digitalocean"
                  data-cat="21"
                  data-av="0"
                  hidden={!s.integrationVisible("digitalocean", "21", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=digitalocean.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"DigitalOcean"}</b>
                    <small>{"Cloud & Infrastructure"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="hugging face"
                  data-cat="24"
                  data-av="0"
                  hidden={!s.integrationVisible("hugging face", "24", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=huggingface.co&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Hugging Face"}</b>
                    <small>{"AI & Agents"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="docusign"
                  data-cat="25"
                  data-av="0"
                  hidden={!s.integrationVisible("docusign", "25", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=docusign.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"DocuSign"}</b>
                    <small>{"E-signature & Legal"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="contentful"
                  data-cat="22"
                  data-av="0"
                  hidden={!s.integrationVisible("contentful", "22", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=contentful.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Contentful"}</b>
                    <small>{"Websites & CMS"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="freshbooks"
                  data-cat="14"
                  data-av="0"
                  hidden={!s.integrationVisible("freshbooks", "14", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=freshbooks.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"FreshBooks"}</b>
                    <small>{"Accounting & ERP"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="wordpress.com"
                  data-cat="22"
                  data-av="0"
                  hidden={!s.integrationVisible("wordpress.com", "22", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=wordpress.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"WordPress.com"}</b>
                    <small>{"Websites & CMS"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="google chat"
                  data-cat="1"
                  data-av="0"
                  hidden={!s.integrationVisible("google chat", "1", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=workspace.google.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Google Chat"}</b>
                    <small>{"Messaging & Telephony"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="workday"
                  data-cat="23"
                  data-av="0"
                  hidden={!s.integrationVisible("workday", "23", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=workday.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Workday"}</b>
                    <small>{"HR, Payroll & Recruiting"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="stack exchange"
                  data-cat="2"
                  data-av="0"
                  hidden={!s.integrationVisible("stack exchange", "2", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=stackexchange.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Stack Exchange"}</b>
                    <small>{"Developer Tools & Code"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="google photos"
                  data-cat="6"
                  data-av="0"
                  hidden={!s.integrationVisible("google photos", "6", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=photos.google.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Google Photos"}</b>
                    <small>{"Files & Storage"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="tiktok ads"
                  data-cat="26"
                  data-av="0"
                  hidden={!s.integrationVisible("tiktok ads", "26", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=ads.tiktok.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"TikTok Ads"}</b>
                    <small>{"Advertising"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="constant contact"
                  data-cat="16"
                  data-av="0"
                  hidden={!s.integrationVisible("constant contact", "16", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=constantcontact.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Constant Contact"}</b>
                    <small>{"Marketing & Email Marketing"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="datadog"
                  data-cat="19"
                  data-av="0"
                  hidden={!s.integrationVisible("datadog", "19", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=datadog.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Datadog"}</b>
                    <small>{"Monitoring & Observability"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="google ads"
                  data-cat="26"
                  data-av="0"
                  hidden={!s.integrationVisible("google ads", "26", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=ads.google.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Google Ads"}</b>
                    <small>{"Advertising"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="zoho crm"
                  data-cat="8"
                  data-av="0"
                  hidden={!s.integrationVisible("zoho crm", "8", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=crm.zoho.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Zoho CRM"}</b>
                    <small>{"CRM & Sales"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="databricks"
                  data-cat="3"
                  data-av="0"
                  hidden={!s.integrationVisible("databricks", "3", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=databricks.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Databricks"}</b>
                    <small>{"Spreadsheets & Databases"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="outline"
                  data-cat="0"
                  data-av="0"
                  hidden={!s.integrationVisible("outline", "0", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=getoutline.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Outline"}</b>
                    <small>{"Docs, Notes & Knowledge"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="vimeo"
                  data-cat="15"
                  data-av="0"
                  hidden={!s.integrationVisible("vimeo", "15", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=vimeo.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Vimeo"}</b>
                    <small>{"Media, Music & Video"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="teamwork"
                  data-cat="7"
                  data-av="0"
                  hidden={!s.integrationVisible("teamwork", "7", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=teamwork.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Teamwork"}</b>
                    <small>{"Project & Task Management"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="microsoft intune"
                  data-cat="28"
                  data-av="0"
                  hidden={!s.integrationVisible("microsoft intune", "28", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=microsoft.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Microsoft Intune"}</b>
                    <small>{"Security & Identity"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="google forms"
                  data-cat="18"
                  data-av="0"
                  hidden={!s.integrationVisible("google forms", "18", "0")}
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
                    <b>{"Google Forms"}</b>
                    <small>{"Forms & Surveys"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="microsoft power bi"
                  data-cat="20"
                  data-av="0"
                  hidden={
                    !s.integrationVisible("microsoft power bi", "20", "0")
                  }
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=microsoft.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Microsoft Power BI"}</b>
                    <small>{"Analytics & BI"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="ringcentral"
                  data-cat="1"
                  data-av="0"
                  hidden={!s.integrationVisible("ringcentral", "1", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=ringcentral.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"RingCentral"}</b>
                    <small>{"Messaging & Telephony"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="copper"
                  data-cat="8"
                  data-av="0"
                  hidden={!s.integrationVisible("copper", "8", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=copper.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Copper"}</b>
                    <small>{"CRM & Sales"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="timely"
                  data-cat="7"
                  data-av="0"
                  hidden={!s.integrationVisible("timely", "7", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=timelyapp.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Timely"}</b>
                    <small>{"Project & Task Management"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="amplitude"
                  data-cat="20"
                  data-av="0"
                  hidden={!s.integrationVisible("amplitude", "20", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=amplitude.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Amplitude"}</b>
                    <small>{"Analytics & BI"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="dropbox sign"
                  data-cat="25"
                  data-av="0"
                  hidden={!s.integrationVisible("dropbox sign", "25", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=hellosign.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Dropbox Sign"}</b>
                    <small>{"E-signature & Legal"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="fireflies"
                  data-cat="9"
                  data-av="0"
                  hidden={!s.integrationVisible("fireflies", "9", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=fireflies.ai&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Fireflies"}</b>
                    <small>{"Video & Meetings"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="zoom phone"
                  data-cat="1"
                  data-av="0"
                  hidden={!s.integrationVisible("zoom phone", "1", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=zoom.us&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Zoom Phone"}</b>
                    <small>{"Messaging & Telephony"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="snapchat"
                  data-cat="26"
                  data-av="0"
                  hidden={!s.integrationVisible("snapchat", "26", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=forbusiness.snapchat.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Snapchat"}</b>
                    <small>{"Advertising"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="ramp"
                  data-cat="11"
                  data-av="0"
                  hidden={!s.integrationVisible("ramp", "11", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=ramp.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Ramp"}</b>
                    <small>{"Payments & Banking"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="freeagent"
                  data-cat="14"
                  data-av="0"
                  hidden={!s.integrationVisible("freeagent", "14", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=freeagent.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"FreeAgent"}</b>
                    <small>{"Accounting & ERP"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="pandadoc"
                  data-cat="25"
                  data-av="0"
                  hidden={!s.integrationVisible("pandadoc", "25", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=pandadoc.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"PandaDoc"}</b>
                    <small>{"E-signature & Legal"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="heroku"
                  data-cat="2"
                  data-av="0"
                  hidden={!s.integrationVisible("heroku", "2", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=heroku.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Heroku"}</b>
                    <small>{"Developer Tools & Code"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="zeplin"
                  data-cat="12"
                  data-av="0"
                  hidden={!s.integrationVisible("zeplin", "12", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=zeplin.io&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Zeplin"}</b>
                    <small>{"Design & Whiteboards"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="brex"
                  data-cat="11"
                  data-av="0"
                  hidden={!s.integrationVisible("brex", "11", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=brex.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Brex"}</b>
                    <small>{"Payments & Banking"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="azure cost management"
                  data-cat="29"
                  data-av="0"
                  hidden={
                    !s.integrationVisible("azure cost management", "29", "0")
                  }
                >
                  <span className="int-logo">{"AC"}</span>
                  <span className="int-txt">
                    <b>{"Azure Cost Management"}</b>
                    <small>{"Cloud cost"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="zoho bigin"
                  data-cat="8"
                  data-av="0"
                  hidden={!s.integrationVisible("zoho bigin", "8", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=bigin.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Zoho Bigin"}</b>
                    <small>{"CRM & Sales"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="roam hq (ro.am)"
                  data-cat="1"
                  data-av="0"
                  hidden={!s.integrationVisible("roam hq (ro.am)", "1", "0")}
                >
                  <span className="int-logo">{"RH"}</span>
                  <span className="int-txt">
                    <b>{"Roam HQ (ro.am)"}</b>
                    <small>{"Messaging & Telephony"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
                <div
                  className="int big"
                  data-name="segment"
                  data-cat="20"
                  data-av="0"
                  hidden={!s.integrationVisible("segment", "20", "0")}
                >
                  <span className="int-logo">
                    <img
                      src="https://www.google.com/s2/favicons?domain=segment.com&sz=64"
                      alt=""
                      loading="lazy"
                      width="20"
                      height="20"
                      onError={s.imageFallback}
                    />
                  </span>
                  <span className="int-txt">
                    <b>{"Segment"}</b>
                    <small>{"Analytics & BI"}</small>
                  </span>
                  <span className="badge b-m">{"Coming soon"}</span>
                </div>
              </div>
            </div>
          </div>
          {"\n  "}
          <div
            className="card"
            style={{
              marginTop: "32px",
              display: "flex",
              justifyContent: "space-between",
              alignItems: "center",
              gap: "20px",
              flexWrap: "wrap",
            }}
          >
            <div style={{ maxWidth: "680px" }}>
              <b style={{ fontSize: "var(--fs-h3)" }}>{"Don’t see yours?"}</b>
              <p
                className="sub"
                style={{ fontSize: "14.5px", marginTop: "6px" }}
              >
                {
                  "Any REST API can be added as a custom service with your own key. Some live integrations still need your own app credentials while managed sign-in rolls out."
                }
              </p>
            </div>
            <a className="btn" href={s.siteHref("/contact")}>
              {"Request an integration "}
              <svg className="i" viewBox="0 0 24 24">
                <path d="M5 12h14M12 5l7 7-7 7"></path>
              </svg>
            </a>
          </div>
          {"\n"}
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap">
          <div className="band">
            <h2 className="h2">{"Hand off the repeat work tonight."}</h2>
            <p style={{ opacity: ".88", marginTop: "12px", fontSize: "17px" }}>
              {
                "Ask NyxBot to set up your first job, connect Telegram, and you’re live."
              }
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
    </>
  );
}
