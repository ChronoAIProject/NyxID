import { ArrowRight, Check } from "lucide-react";
import copy from "./pricing-copy.json";
import {
  StoryDetails,
  StoryHero,
  StoryLink,
  StoryNext,
  StoryPage,
  StorySection,
  StorySplit,
} from "./story-page";
import { useWebsite } from "./website-context";
import "./commercial-story.css";

const SELF_HOST_URL = "https://github.com/ChronoAIProject/NyxID";

// Effective pricing policy: chrono-finance#315, decisions D2, D3 and D5.
// Every figure and policy sentence comes from pricing-copy.json; only short
// structural labels (who pays whom) are written here, in both languages.
export function PricingSection() {
  const s = useWebsite();
  const c = copy[s.language];
  const zh = s.language === "zh";
  // cards: we pay providers, your own accounts, then prepaid credits.
  const paths = c.cards.slice(0, 2);
  const credits = c.cards[2];
  // Who sends the bill on each path, in the same words as the card text.
  const flows = [
    zh ? ["你的积分", "NyxID 代付服务商"] : ["Your credits", "NyxID pays the provider"],
    zh ? ["你的服务商账号", "服务商直接向你收费"] : ["Your provider account", "The provider bills you"],
  ];

  return (
    <StoryPage className="st-commercial stc-pricing">
      <StoryHero
        title={
          <>
            {c.heading}
            <br />
            <em>{c.highlight}</em>
          </>
        }
        description={c.description}
        visual={
          <figure className="stc-receipt">
            <div className="stc-ticket-wrap">
              <div className="stc-ticket">
                <p className="stc-ticket-head" id="pricing-example">
                  {c.exampleLabel}
                </p>
                <dl className="stc-lines" aria-labelledby="pricing-example">
                  {c.exampleRows.map((row) => (
                    <div key={row.label} className={row.total ? "is-total" : undefined}>
                      <dt>{row.label}</dt>
                      <dd>{row.amount}</dd>
                    </div>
                  ))}
                </dl>
              </div>
            </div>
            <figcaption>{c.exampleNote}</figcaption>
          </figure>
        }
      >
        <StoryLink href={s.authHref("register")} primary>
          {c.start}
        </StoryLink>
      </StoryHero>

      <StorySection
        id="price-basis"
        title={
          <>
            {c.commitmentHeading} <em>{c.commitmentHighlight}</em>
          </>
        }
        description={c.commitmentDescription}
      >
        <ul className="stc-basis">
          {c.checks.map((check) => (
            <li key={check.title}>
              <Check size={18} aria-hidden="true" />
              <h3>{check.title}</h3>
              <p>{check.text}</p>
            </li>
          ))}
        </ul>
      </StorySection>

      <StorySection id="paying-providers" title={c.pricingHeading}>
        <div className="stc-choice">
          {paths.map((card, index) => (
            <article className="stc-path" key={card.title}>
              <span className="stc-pill">{card.badge}</span>
              <h3>{card.title}</h3>
              <p>{card.text}</p>
              <ol className="stc-flow" aria-label={card.badge}>
                {(flows[index] ?? []).map((step, position) => (
                  <li key={step}>
                    {position > 0 && <ArrowRight size={15} aria-hidden="true" />}
                    <span>{step}</span>
                  </li>
                ))}
              </ol>
            </article>
          ))}
        </div>
        <div className="stc-credit">
          {credits && (
            <div>
              <span className="stc-pill">{credits.badge}</span>
              <h3>{credits.title}</h3>
              <p>{credits.text}</p>
            </div>
          )}
          <div className="stc-self-host">
            <p>{c.freeNote}</p>
            <StoryLink href={SELF_HOST_URL}>
              {zh ? "用 Docker 自托管" : "Self-host with Docker"}
            </StoryLink>
          </div>
        </div>
      </StorySection>

      <StorySection id="pricing-questions">
        <StorySplit className="stc-faq">
          <div>
            <h2>{c.faqHeading}</h2>
          </div>
          <div>
            {c.faqs.map((faq) => (
              <StoryDetails key={faq.question} summary={faq.question}>
                <p>{faq.answer}</p>
              </StoryDetails>
            ))}
          </div>
        </StorySplit>
      </StorySection>

      {/* The pricing path ends with the people who can answer what the policy does not. */}
      <StoryNext
        title={zh ? "关于套餐或自托管还有问题？" : "Questions about bundles or self-hosting?"}
        href={s.siteHref("/contact")}
        label={c.talk}
      />
    </StoryPage>
  );
}
