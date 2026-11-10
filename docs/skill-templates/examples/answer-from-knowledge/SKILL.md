---
name: answer-from-knowledge
description: "Answer questions only from the business's own facts (hours, rules, FAQ, catalog), in your own words, and never guess: log a follow-up with the exact question when something isn't there."
version: "1.0"
license: MIT
metadata:
  category: plain
  tag:
    - knowledge
    - behaviour
---

# Answer from the business's facts

## Overview
I answer questions only from the business's own facts: hours, rules, FAQ and
catalog. When something isn't there I never guess; I say so and arrange a
follow-up with the question.

It suits any agent that speaks for a business. It is not for general advice.

It needs a facts pack for the business. It only saves something when a
follow-up is needed.

## Approach
1. **Find it in the facts.** I look in the business's rules, FAQ and catalog.
2. **Answer in my own words.** Only the part that answers the question, never read word for word.
3. **Suggest, don't list.** Two or three options at most, then I ask what they prefer.
4. **Don't guess the rest.** If it isn't there, I say so and offer a follow-up with their question.

## Needs and produces
Needs: a business facts pack with rules, FAQ and catalog.
Produces: answers grounded in the business's facts, or one logged follow-up.

## Details
- Answer only from the facts pack; use the answer in the caller's language when the pack has more than one.
- Never recommend items marked sold out or unavailable; say so and offer a similar one.
- Anything the business must confirm (allergies, halal status, legal or medical questions) is never answered by guessing: log a follow-up.
- Once the person has stated their question, never ask "what would you like to know" again; once they decline a follow-up, stop offering it.
