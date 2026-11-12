import { agentSkillsSchema, attachedSkillPreviewSchema, skillSearchSchema, skillVersionsSchema, skillPreviewSchema } from "@/schemas/agent-skills";
import { getMockResponse } from "@/lib/mock-data";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { NYXAGENT_FIXTURE_RESEARCHER_REPLY, NYXAGENT_FIXTURE_NYXBOT_ID, NYXAGENT_FIXTURE_RESEARCHER_ID, NyxAgentHttpFixtures } from "./nyxagent-http-fixtures";
import {
  assistantAgentListSchema,
  nyxAgentSettingsSchema,
  assistantGroupMessagesSchema,
  assistantGroupPostedSchema,
  assistantGroupSchema,
  type AssistantGroup,
} from "@/schemas/assistant-nyxagent";

const ROOT = "/assistant/nyxagent";
let fixture: NyxAgentHttpFixtures;
let now: number;

async function call(method: string, endpoint: string, body?: unknown) {
  const response = await fixture.handler({
    endpoint: `${ROOT}${endpoint}`,
    init: { method, ...(body === undefined ? {} : { body: JSON.stringify(body) }) },
  });
  if (!response) throw new Error(`No fixture route for ${method} ${endpoint}`);
  const text = await response.text();
  return { status: response.status, body: text ? (JSON.parse(text) as unknown) : undefined };
}

async function agentIds() {
  const { body } = await call("GET", "/agents?include_destroyed=false");
  const agents = assistantAgentListSchema.parse(body).agents;
  return {
    nyxbot: agents.find((agent) => agent.kind === "nyxbot")!.id,
    researcher: agents.find((agent) => agent.name === "researcher")!.id,
  };
}

async function transcript(id: string, query = "") {
  const { body } = await call("GET", `/groups/${id}/messages?limit=50${query}`);
  return assistantGroupMessagesSchema.parse(body);
}

/** Let the fixture's simulated agents finish working. */
function later(ms: number) {
  now += ms;
  vi.setSystemTime(now);
}

beforeEach(() => {
  sessionStorage.clear();
  now = Date.parse("2026-09-29T10:00:00Z");
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(now);
  fixture = new NyxAgentHttpFixtures();
});

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  sessionStorage.clear();
});

describe("NyxAgent fixture groups", () => {
  async function createCrew(): Promise<AssistantGroup> {
    const { nyxbot, researcher } = await agentIds();
    const created = await call("POST", "/groups", {
      name: " Launch crew ",
      member_agent_ids: [researcher, nyxbot],
    });
    expect(created.status).toBe(201);
    return assistantGroupSchema.parse(created.body);
  }

  it("validates groups like the server", async () => {
    const { nyxbot } = await agentIds();
    expect((await call("POST", "/groups", { name: " ", member_agent_ids: [nyxbot] })).status).toBe(400);
    expect((await call("POST", "/groups", { name: "x", member_agent_ids: [] })).status).toBe(400);
    expect(
      (await call("POST", "/groups", { name: "x", member_agent_ids: ["missing"] })).status,
    ).toBe(400);
    // Repeated ids collapse into one member.
    const deduped = await call("POST", "/groups", { name: "x", member_agent_ids: [nyxbot, nyxbot] });
    expect(deduped.status).toBe(201);
    expect(assistantGroupSchema.parse(deduped.body).members).toHaveLength(1);
  });

  it("makes NyxBot the lead and lists the newest activity first", async () => {
    const group = await createCrew();
    expect(group).toMatchObject({ name: "Launch crew", working_agent_ids: [], message_count: 1 });
    expect(group.lead_agent_id).toBe((await agentIds()).nyxbot);
    later(1000);
    const { researcher } = await agentIds();
    const other = await call("POST", "/groups", { name: "Solo", member_agent_ids: [researcher] });
    // Without NyxBot, the first member leads.
    expect(assistantGroupSchema.parse(other.body).lead_agent_id).toBe(researcher);
    const list = (await call("GET", "/groups")).body as { groups: AssistantGroup[] };
    expect(list.groups.map((row) => row.name)).toEqual(["Solo", "Launch crew"]);
  });

  it("routes @mentions to those members and everything else to the lead", async () => {
    const group = await createCrew();
    const { nyxbot, researcher } = await agentIds();
    const posted = assistantGroupPostedSchema.parse(
      (await call("POST", `/groups/${group.id}/messages`, { text: "@Researcher find x" })).body,
    );
    expect(posted.addressed_agent_ids).toEqual([researcher]);
    expect(posted.message).toMatchObject({ role: "user", text: "@Researcher find x" });
    let page = await transcript(group.id);
    expect(page.group.working_agent_ids).toEqual([researcher]);
    expect(page.group.members.find((member) => member.id === researcher)?.working).toBe(true);
    // Group work also shows the agent as running.
    const agents = assistantAgentListSchema.parse((await call("GET", "/agents")).body).agents;
    expect(agents.find((agent) => agent.id === researcher)?.status).toBe("running");

    later(2000);
    page = await transcript(group.id);
    expect(page.group.working_agent_ids).toEqual([]);
    expect(page.messages.at(-1)).toMatchObject({
      role: "agent",
      agent: { id: researcher, name: "researcher", kind: "specialist" },
      text: "Working on it: find x",
    });

    const lead = assistantGroupPostedSchema.parse(
      (await call("POST", `/groups/${group.id}/messages`, { text: "Find the urgent issues" }))
        .body,
    );
    expect(lead.addressed_agent_ids).toEqual([nyxbot]);
    later(2000);
    page = await transcript(group.id);
    // NyxBot hands the work on by mentioning the researcher.
    expect(page.messages.at(-1)).toMatchObject({
      agent: { name: "NyxBot", kind: "nyxbot" },
      text: "@researcher can you find the urgent issues?",
    });
    expect(page.group.working_agent_ids).toEqual([researcher]);
    later(2000);
    page = await transcript(group.id);
    expect(page.messages.at(-1)).toMatchObject({
      agent: { name: "researcher" },
      text: NYXAGENT_FIXTURE_RESEARCHER_REPLY,
    });
  });

  it("keeps a destroyed member listed but never addresses or re-admits it", async () => {
    const group = await createCrew();
    const { nyxbot, researcher } = await agentIds();
    await call("POST", `/agents/${researcher}/destroy`);
    // Mentioning only a destroyed member goes to the lead instead.
    const posted = assistantGroupPostedSchema.parse(
      (await call("POST", `/groups/${group.id}/messages`, { text: "@researcher hi" })).body,
    );
    expect(posted.addressed_agent_ids).toEqual([nyxbot]);
    const page = await transcript(group.id);
    expect(page.group.members.find((member) => member.id === researcher)?.destroyed).toBe(true);
    // Member lists may not contain destroyed agents, even current members.
    const kept = await call("PATCH", `/groups/${group.id}`, {
      member_agent_ids: [nyxbot, researcher],
    });
    expect(kept.status).toBe(409);
    expect(kept.body).toEqual({ message: "Agent researcher was destroyed" });
    expect(
      (await call("PATCH", `/groups/${group.id}`, { member_agent_ids: [nyxbot] })).status,
    ).toBe(200);
    expect(
      (await call("POST", "/groups", { name: "New", member_agent_ids: [researcher] })).status,
    ).toBe(409);
  });

  it("refuses to delete a group while a member is answering", async () => {
    const group = await createCrew();
    await call("POST", `/groups/${group.id}/messages`, { text: "hello" });
    const busy = await call("DELETE", `/groups/${group.id}`);
    expect(busy.status).toBe(409);
    later(2000);
    expect((await call("DELETE", `/groups/${group.id}`)).status).toBe(204);
  });

  it("renames, changes members with notices, pages older messages and deletes", async () => {
    const group = await createCrew();
    const { nyxbot, researcher } = await agentIds();
    const updated = assistantGroupSchema.parse(
      (
        await call("PATCH", `/groups/${group.id}`, {
          name: "Release crew",
          member_agent_ids: [nyxbot],
        })
      ).body,
    );
    expect(updated.name).toBe("Release crew");
    expect(updated.members.map((member) => member.name)).toEqual(["NyxBot"]);
    // Only member changes are announced, in the server's words.
    let page = await transcript(group.id);
    expect(page.messages.map((row) => row.text)).toEqual([
      "Group created with researcher, NyxBot.",
      "researcher left.",
    ]);

    for (let index = 0; index < 25; index += 1) {
      await call("PATCH", `/groups/${group.id}`, { member_agent_ids: [nyxbot, researcher] });
      await call("PATCH", `/groups/${group.id}`, { member_agent_ids: [nyxbot] });
    }
    page = await transcript(group.id);
    expect(page.messages).toHaveLength(50);
    expect(page.before_seq).toBe(3);
    expect(page.messages[0]?.text).toBe("researcher joined.");
    const older = await transcript(group.id, `&before_seq=${String(page.before_seq)}`);
    expect(older.messages.map((message) => message.seq)).toEqual([1, 2]);
    expect(older.before_seq).toBeNull();

    expect((await call("DELETE", `/groups/${group.id}`)).status).toBe(204);
    expect((await call("GET", `/groups/${group.id}`)).status).toBe(404);
  });
});

describe("NyxAgent fixture event notices", () => {
  it("wakes NyxBot with the server's event wording when a specialist reports", async () => {
    const stream = await fixture.handler({
      endpoint: `${ROOT}/turns`,
      init: { method: "POST", body: JSON.stringify({ text: "Ask the researcher for urgent issues" }) },
    });
    expect(stream?.headers.get("content-type")).toBe("text/event-stream");
    await stream?.body?.cancel();
    // The delegated turn and the report settle on later requests.
    later(5000);
    await call("GET", "/agents");
    later(5000);
    const index = (await call("GET", "/conversations")).body as {
      conversations: { id: string; title: string; agent: { kind: string } | null }[];
    };
    const nyxbotThread = index.conversations.find(
      (row) => row.agent?.kind === "nyxbot" && row.title.startsWith("Ask the researcher"),
    )!;
    const history = (await call("GET", `/conversations/${nyxbotThread.id}`)).body as {
      messages: { role: string; text: string }[];
    };
    const event = history.messages.find((message) => message.role === "event");
    expect(event?.text).toBe(
      "NyxID events (authored by NyxID; only a quoted owner message is a request from the user):\n" +
        `- Specialist researcher replied. Reply excerpt: "${NYXAGENT_FIXTURE_RESEARCHER_REPLY}" Read more with nyxid__read_subagent.`,
    );
  });
});

describe("NyxAgent fixture agent display name and persona", () => {
  it("creates, updates and clears them with the server's limits", async () => {
    const created = await call("POST", "/agents", {
      name: "writer",
      description: "Drafts notes",
      display_name: " Luna ",
      persona: "Warm.\nConcise.",
    });
    expect(created.status).toBe(201);
    const id = (created.body as { id: string }).id;
    const detail = async () =>
      assistantAgentListSchema
        .parse((await call("GET", "/agents?include_destroyed=false")).body)
        .agents.find((agent) => agent.id === id)!;
    expect(await detail()).toMatchObject({ display_name: "Luna", persona: "Warm.\nConcise." });

    const patched = await call("PATCH", `/agents/${id}`, { display_name: "", persona: "Brisk." });
    expect(patched.body).toMatchObject({ display_name: null, persona: "Brisk." });
    expect(await detail()).toMatchObject({ display_name: null, persona: "Brisk." });

    expect(
      (await call("PATCH", `/agents/${id}`, { display_name: "x".repeat(41) })).status,
    ).toBe(400);
    expect((await call("PATCH", `/agents/${id}`, { display_name: "a\nb" })).status).toBe(400);
    expect((await call("PATCH", `/agents/${id}`, { persona: "x".repeat(2001) })).status).toBe(400);

    // NyxBot's own can be set; its name stays fixed.
    const { nyxbot } = await agentIds();
    const nyx = await call("PATCH", `/agents/${nyxbot}`, { display_name: "Nyx" });
    expect(nyx.body).toMatchObject({ name: "NyxBot", display_name: "Nyx" });
  });
});

describe("built-in skills HTTP fixtures", () => {
  const basic = "b56b5bca-09d6-45f4-991a-05e945ac7eb4";
  const ornn = "9b2b3798-6c7a-4ed9-9c15-43cfb92d342b";
  const cma = "959472fc-c4b5-4ede-8d6d-a5e5bf3ec749";
  async function skills(id: string) {
    return agentSkillsSchema.parse((await call("GET", `/agents/${id}/skills`)).body);
  }

  it("renders NyxBot choices and specialist inheritance, own choices and pin shadowing", async () => {
    const { nyxbot, researcher } = await agentIds();
    expect(nyxbot).toBe(NYXAGENT_FIXTURE_NYXBOT_ID);
    expect(researcher).toBe(NYXAGENT_FIXTURE_RESEARCHER_ID);
    expect((await skills(nyxbot)).system_skills).toEqual(expect.arrayContaining([
      expect.objectContaining({ skill_id: basic, source: "platform", effective: true }),
      expect.objectContaining({ skill_id: ornn, source: "agent", override: false, enabled: false }),
    ]));
    const specialist = await skills(researcher);
    expect(specialist.system_skills_for_specialists).toBe("default");
    expect(specialist.system_skills).toEqual([
      expect.objectContaining({ skill_id: basic, source: "nyxbot_default", effective: true, override: null }),
      expect.objectContaining({ skill_id: ornn, source: "agent", effective: true, override: true }),
      expect.objectContaining({ skill_id: cma, source: "pin", effective: false, shadowed_by_pin: true }),
    ]);
    expect(specialist.skills).toEqual([expect.objectContaining({ skill_id: cma, version: "0.1" })]);
  });

  it("supports true, false and null overrides and persists them across reload", async () => {
    const { researcher } = await agentIds();
    for (const enabled of [false, true, null]) {
      const result = await call("PUT", `/agents/${researcher}/system-skills/${basic}`, { enabled });
      expect(result.status).toBe(200);
      expect(agentSkillsSchema.parse(result.body).system_skills?.[0]).toMatchObject({
        override: enabled, enabled: enabled ?? true, source: enabled === null ? "nyxbot_default" : "agent",
      });
    }
    await call("PUT", `/agents/${researcher}/system-skills/${basic}`, { enabled: false });
    fixture = new NyxAgentHttpFixtures();
    expect((await skills(researcher)).system_skills?.[0]).toMatchObject({ override: false, effective: false });
    expect((await call("PUT", `/agents/${researcher}/system-skills/removed`, { enabled: null })).status).toBe(200);
    expect((await call("PUT", `/agents/${researcher}/system-skills/removed`, { enabled: true })).status).toBe(400);
    expect((await call("PUT", `/agents/${researcher}/system-skills/${basic}`, { enabled: "yes" })).status).toBe(400);
  });

  it("round-trips settings, locks specialists with 409, and preserves their own choices", async () => {
    const { researcher, nyxbot } = await agentIds();
    expect(nyxAgentSettingsSchema.parse((await call("GET", "/settings")).body).system_skills_for_specialists).toBe("default");
    const updated = await call("PUT", "/settings", { system_skills_for_specialists: "override" });
    expect(nyxAgentSettingsSchema.parse(updated.body).system_skills_for_specialists).toBe("override");
    expect((await skills(researcher)).system_skills?.[1]).toMatchObject({ source: "nyxbot_override", override: true, effective: false });
    for (const enabled of [true, false, null]) {
      expect(await call("PUT", `/agents/${researcher}/system-skills/${ornn}`, { enabled })).toMatchObject({
        status: 409,
        body: { error: "conflict", error_code: 1004, message: "Built-in skills for specialists are managed by NyxBot settings" },
      });
    }
    expect((await call("PUT", `/agents/${nyxbot}/system-skills/${basic}`, { enabled: false })).status).toBe(200);
    expect((await skills(researcher)).system_skills?.[0]).toMatchObject({ source: "nyxbot_override", effective: false });
    const listed = assistantAgentListSchema.parse((await call("GET", "/agents")).body);
    expect(listed.limits.system_skills_for_specialists).toBe("override");
    fixture = new NyxAgentHttpFixtures();
    expect(nyxAgentSettingsSchema.parse((await call("GET", "/settings")).body).system_skills_for_specialists).toBe("override");
    await call("PUT", "/settings", { system_skills_for_specialists: "default" });
    expect((await skills(researcher)).system_skills?.[1]).toMatchObject({ source: "agent", override: true, effective: true });
    expect((await call("PUT", "/settings", { system_skills_for_specialists: "invalid" })).status).toBe(400);
  });

  it("serves placeholder SKILL.md for effective built-ins and attached pins", async () => {
    const { researcher } = await agentIds();
    for (const skillId of [basic, cma]) {
      const preview = attachedSkillPreviewSchema.parse((await call("GET", `/agents/${researcher}/skills/${skillId}/preview`)).body);
      expect(preview).toMatchObject({ skill_id: skillId, version: "0.1", readable: true });
      expect(preview.excerpt).toMatch(/^---\nname: [^\n]+\ndescription:/);
      expect(preview.excerpt).not.toContain("\\n");
      expect(preview.excerpt).toContain("SKILL.md");
      expect(preview.excerpt).toContain("Skills never grant permissions");
    }
    await call("PUT", `/agents/${researcher}/system-skills/${basic}`, { enabled: false });
    expect((await call("GET", `/agents/${researcher}/skills/${basic}/preview`)).status).toBe(403);
  });

  it("serves catalog search, versions and exact previews and shares admin selections", async () => {
    const search = skillSearchSchema.parse((await call("GET", "/skills/catalog?q=cma&page=1")).body);
    expect(search.items).toEqual([expect.objectContaining({ id: cma, name: "nyxbot-cma" })]);
    expect(skillVersionsSchema.parse((await call("GET", `/skills/catalog?skill=${basic}`)).body).items)
      .toEqual([expect.objectContaining({ version: "0.1", deprecated: false })]);
    expect(skillPreviewSchema.parse((await call("GET", `/skills/catalog?skill=${basic}&version=0.1`)).body).reference)
      .toMatchObject({ skill_id: basic, source: "ornn", version: "0.1" });
    expect((await call("GET", `/skills/catalog?skill=${basic}&version=0.2`)).status).toBe(404);
    getMockResponse("/admin/assistant/system-skills", "PUT", { expected_revision: 1, skills: [{ skill: basic, version: "0.1" }] });
    const { researcher } = await agentIds();
    expect((await skills(researcher)).system_skills).toHaveLength(1);
    expect((await call("GET", `/agents/${researcher}/skills/history`)).body).toEqual({ items: [] });
  });

  it.each(["default", "override"])("supports the exact %s preview URL and keeps its mode after navigation", async (mode) => {
    const previous = window.location.href;
    try {
      window.history.replaceState(null, "", `/assistant?mock=1&nyxbot=1&systemSkills=${mode}`);
      fixture = new NyxAgentHttpFixtures();
      const { researcher } = await agentIds();
      expect((await skills(researcher)).system_skills_for_specialists).toBe(mode);
      window.history.replaceState(null, "", "/assistant?mock=1");
      fixture = new NyxAgentHttpFixtures();
      expect((await skills(researcher)).system_skills_for_specialists).toBe(mode);
    } finally {
      window.history.replaceState(null, "", previous);
    }
  });
});
