import {
  compileScenarioSet,
  flow,
  scenario,
} from "@/lib/assistant/scenario-engine";

export const flows = {
  "connect-github": flow((script) =>
    script
      .say("I need access to your **GitHub** account first.")
      .action("service.connect", {
        service: "api-github",
        scopes: ["repo", "read:org"],
      })
      .await({
        declined: (branch) =>
          branch
            .say("No problem — I'll skip anything that needs GitHub.")
            .stop(),
        failed: (branch) =>
          branch
            .say("The connection didn't complete. Ask me again when ready.")
            .stop(),
      })
      .connect("api-github")
      .say(
        "GitHub connected — credential sealed in NyxID, never shared with the model.",
      ),
  ),
};

export const scenarios = [
  scenario("connect-github", /connect (to )?(my )?github/i, (script) =>
    script
      .whenConnected("api-github", (branch) =>
        branch.say("You're already connected to GitHub.").stop(),
      )
      .run("connect-github"),
  ),
  scenario(
    "github-issues",
    /(what|show|list).*(gh|github).*issues/i,
    (script) =>
      script
        .need("api-github", "connect-github")
        .tool("github.listIssues", "7 open, 2 stale")
        .say(
          "You have **7 open issues** on `acme/web`. Two haven't moved in 30 days.",
        ),
  ),
  scenario("github-issues-repo", /issues (?:in|on) (\S+)/i, (script, match) =>
    script
      .need("api-github", "connect-github")
      .tool(`github.listIssues repo=${match[1]}`, "3 open")
      .say(`\`${match[1]}\` has **3 open issues**.`),
  ),
  scenario("approval-demo", /post .*digest/i, (script) =>
    script
      .tool("github.buildDigest", "Digest ready")
      .approval("api-github", "Post the issue digest to the connected channel?")
      .await({
        approved: (branch) => branch.say("The digest was approved and posted."),
        denied: (branch) => branch.say("The digest was not posted."),
      }),
  ),
  scenario("error-demo", /simulate (an? )?error/i, (script) =>
    script
      .say("Starting the scripted error rehearsal.")
      .fail("mock_error", "The mock scenario failed as requested."),
  ),
];

export const compiledScenarios = compileScenarioSet(scenarios, flows);
