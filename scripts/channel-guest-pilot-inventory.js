/* global db, EJSON, print */
// Run with mongosh --norc --quiet --file and a read-only database account.
const requested = (process.env.NYXID_PILOT_BOT_IDS ?? "").split(",").map((id) => id.trim()).filter(Boolean);
const botIds = [...new Set(requested)];
const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
if (botIds.length < 1 || botIds.length > 100 || botIds.some((id) => !uuid.test(id))) {
  throw new Error("Set NYXID_PILOT_BOT_IDS to 1–100 comma-separated bot UUIDs");
}

const limit = 2000;
const now = new Date();

function bounded(rows, collection) {
  if (rows.length > limit) throw new Error(`Too many ${collection} rows. Report on a smaller cohort.`);
  return rows;
}

function read(collection, filter, projection) {
  return bounded(
    db.getCollection(collection).find(filter, projection).limit(limit + 1).maxTimeMS(5000).toArray(),
    collection,
  );
}

function readMetadata(collection, filter, projection) {
  return bounded(db.getCollection(collection).aggregate([
    { $match: filter }, { $limit: limit + 1 }, { $project: projection },
  ], { maxTimeMS: 5000 }).toArray(), collection);
}

function unique(values) {
  return [...new Set(values.filter(Boolean))].sort();
}

const bots = read("channel_bots", { _id: { $in: botIds } }, {
  _id: 1, user_id: 1, platform: 1, status: 1, is_active: 1, ownership_version: 1,
  guest_access_v1: 1, guest_access_v1_revision: 1,
  "guest_skill_selection.agent_id": 1, "guest_skill_selection.skills_revision": 1,
  "guest_skill_selection.attachments.skill_id": 1, "guest_skill_selection.attachments.audiences": 1,
  "guest_skill_selection.attachments.all_requests_approved": 1,
});
if (bots.length !== botIds.length) throw new Error("Some requested bot UUIDs were not found");

const hostedProjection = {
  _id: 1, user_id: 1, channel_bot_id: 1, bot_owner_id: 1, status: 1, transport: 1, agent_id: 1,
  route_id: 1, route_api_key_id: 1, private_chats: 1,
  "org_specialist.org_id": 1, "org_specialist.guests.enabled": 1,
  "org_specialist.guests.follows_agent_grants": 1, "org_specialist.guests.service_ids": 1,
};
const hosted = read("nyxbot_channels", {
  channel_bot_id: { $in: botIds }, status: { $in: ["active", "pending"] },
}, hostedProjection);
const threads = read("nyxbot_threads", { channel_id: { $in: hosted.map((row) => row._id) } }, {
  _id: 1, channel_id: 1, agent_id: 1, kind: 1, members: 1, owner_seen: 1,
});
const agentIds = unique([...hosted, ...threads].map((row) => row.agent_id));
const defaults = unique(hosted.filter((row) => !row.agent_id && !row.org_specialist).map((row) => row.user_id));
function selectionMetadata(field) {
  return { $map: {
    input: { $objectToArray: { $ifNull: [field, {}] } },
    as: "scope",
    in: {
      scope_id: "$$scope.k",
      selected: { $map: {
        input: { $ifNull: ["$$scope.v.operations", []] },
        as: "operation",
        in: { $ifNull: ["$$operation.endpoint_id", null] },
      } },
    },
  } };
}
const agents = readMetadata("assistant_agents", {
  destroyed_at: null,
  $or: [{ _id: { $in: agentIds } }, { kind: "nyxbot", user_id: { $in: defaults } }],
}, {
  _id: 1, user_id: 1, kind: 1, skills_revision: 1,
  "skills.skill_id": 1, "skills.source": 1, "skills.version": 1, "skills.sha256": 1,
  "skills.disabled": 1, "skills.dependencies.skill_id": 1,
  "skills.dependencies.version": 1, "skills.dependencies.sha256": 1,
  "grants.service_ids": 1, "grants.platform_service_ids": 1, guest_access: 1,
  guest_operation_scope_revisions: 1, operation_scope_revisions: 1,
  guest_operations: selectionMetadata("$guest_operation_scopes"),
  ordinary_operations: selectionMetadata("$operation_scopes"),
});

const serviceIds = unique(agents.flatMap((agent) => agent.grants?.service_ids ?? []));
const services = read("user_services", { _id: { $in: serviceIds } }, {
  _id: 1, user_id: 1, catalog_service_id: 1, slug: 1, is_active: 1, deleted_at: 1,
});
const routeProjection = {
  _id: 1, user_id: 1, channel_bot_id: 1, agent_api_key_id: 1,
  default_agent: 1, allow_agent_initiated: 1, platform_conversation_type: 1,
};
const routes = read("channel_conversations", { channel_bot_id: { $in: botIds }, is_active: true }, routeProjection);
const keyIds = unique(routes.map((route) => route.agent_api_key_id));
const activeAssignments = read("channel_conversations", {
  agent_api_key_id: { $in: keyIds }, is_active: true,
}, { _id: 1, agent_api_key_id: 1, channel_bot_id: 1 });
const hostedAssignments = read("nyxbot_channels", {
  $or: [{ route_api_key_id: { $in: keyIds } }, { pending_route_api_key_id: { $in: keyIds } }],
}, { _id: 1, route_api_key_id: 1, pending_route_api_key_id: 1 });

const keys = readMetadata("api_keys", { _id: { $in: keyIds } }, {
    _id: 1, user_id: 1, is_active: 1, expires_at: 1, purpose: 1, guest_relay_bot_id: 1,
    callback_configured: { $cond: [
      { $eq: [{ $type: "$callback_url" }, "string"] }, { $ne: ["$callback_url", ""] }, false,
    ] },
    allow_all_services: { $ifNull: ["$allow_all_services", true] },
    allow_all_nodes: { $ifNull: ["$allow_all_nodes", true] },
    allow_auto_connected_services: { $ifNull: ["$allow_auto_connected_services", false] },
    scoped_services: { $size: { $ifNull: ["$allowed_service_ids", []] } },
    scoped_nodes: { $size: { $ifNull: ["$allowed_node_ids", []] } },
});

const owners = read("users", { _id: { $in: unique(bots.map((bot) => bot.user_id)) } }, {
  _id: 1, user_type: 1, is_active: 1, platform_personal_deleting: 1,
});
const orgIds = owners.filter((owner) => owner.user_type === "org").map((owner) => owner._id);
const memberships = read("org_memberships", {
  org_user_id: { $in: orgIds }, role: "admin", revoked_at: null,
}, { _id: 0, org_user_id: 1, member_user_id: 1 });
const recipients = read("users", {
  _id: { $in: unique([...memberships.map((row) => row.member_user_id), ...bots.map((bot) => bot.user_id)]) },
  is_active: true, user_type: { $ne: "org" }, platform_personal_deleting: { $ne: true },
}, { _id: 1 });
const activeRecipients = new Set(recipients.map((person) => person._id));

function externalRoute(bot, route) {
  const key = keys.find((row) => row._id === route.agent_api_key_id);
  const otherRoute = activeAssignments.some((row) => row.agent_api_key_id === route.agent_api_key_id
    && row.channel_bot_id !== bot._id);
  const assignedToHosted = hostedAssignments.some((row) =>
    row.route_api_key_id === route.agent_api_key_id || row.pending_route_api_key_id === route.agent_api_key_id);
  const blocked = [];
  if (!bot.is_active) blocked.push("inactive_bot");
  if (route.user_id !== bot.user_id) blocked.push("route_owner_mismatch");
  if (!key?.is_active || key.user_id !== bot.user_id || (key.expires_at && key.expires_at <= now)) {
    blocked.push("missing_inactive_expired_or_mismatched_key");
  }
  if (!key?.callback_configured) blocked.push("callback_missing");
  if (otherRoute) blocked.push("key_on_another_active_bot_or_device_route");
  if (assignedToHosted) blocked.push("key_on_hosted_or_pending_assignment");
  if (key?.guest_relay_bot_id && key.guest_relay_bot_id !== bot._id) blocked.push("key_retained_for_another_bot");
  return {
    route_id: route._id, default_route: route.default_agent === true,
    destination_kind: route.platform_conversation_type, initiated_posts: route.allow_agent_initiated === true,
    assigned_key_id: route.agent_api_key_id, key_purpose: key?.purpose ?? "general",
    callback_configured: key?.callback_configured ?? false,
    retained_for_this_bot: key?.guest_relay_bot_id === bot._id,
    existing_key_access: key ? {
      all_services: key.allow_all_services, all_nodes: key.allow_all_nodes,
      auto_connected_services: key.allow_auto_connected_services, scoped_services: key.scoped_services,
      scoped_nodes: key.scoped_nodes,
    } : null,
    dedicated_key_candidate: blocked.length === 0, blockers: blocked,
  };
}

function agentInventory(agent) {
  const selection = (operations, id, catalogId) => {
    const applicable = operations.filter((row) => row.scope_id === id || row.scope_id === catalogId);
    return applicable.map((scope) => ({
      scope_id: scope.scope_id,
      endpoint_ids: unique(scope.selected),
      explicit_rule_count: scope.selected.filter((id) => id === null).length,
    }));
  };
  return {
    agent_id: agent._id, kind: agent.kind, skills_revision: agent.skills_revision ?? 0,
    pins: agent.skills ?? [],
    guest_service_grants: (agent.grants?.service_ids ?? []).map((id) => {
      const service = services.find((row) => row._id === id && row.user_id === agent.user_id);
      const catalogId = service?.catalog_service_id;
      return {
        service_id: id, service_slug: service?.slug ?? null,
        active: service?.is_active === true && !service.deleted_at,
        ordinary_guest_level: agent.guest_access?.[id] ?? agent.guest_access?.[catalogId] ?? "use",
        guest_operation_selection: selection(agent.guest_operations, id, catalogId),
        ordinary_operation_selection: selection(agent.ordinary_operations, id, catalogId),
      };
    }),
    platform_service_ids: agent.grants?.platform_service_ids ?? [],
  };
}

const report = bots.map((bot) => {
  const bindings = hosted.filter((row) => row.channel_bot_id === bot._id);
  const owner = owners.find((row) => row._id === bot.user_id);
  const people = owner?.user_type === "org" && !owner.platform_personal_deleting && owner.is_active
    ? memberships.filter((row) => row.org_user_id === bot.user_id).map((row) => row.member_user_id)
    : [bot.user_id];
  return {
    bot_id: bot._id, platform: bot.platform, active: bot.is_active === true, status: bot.status,
    ownership_version: bot.ownership_version ?? 0,
    guest_access_v1: bot.guest_access_v1 === true,
    guest_access_v1_revision: bot.guest_access_v1_revision ?? 0,
    owner_notice_recipient_ids: unique(people.filter((id) => activeRecipients.has(id))),
    hosted_bindings: bindings.map((binding) => {
      const defaultAgent = binding.agent_id
        ? agents.find((agent) => agent._id === binding.agent_id)
        : agents.find((agent) => agent.kind === "nyxbot" && agent.user_id === binding.user_id);
      const chatRows = threads.filter((row) => row.channel_id === binding._id);
      const reachable = unique([defaultAgent?._id, ...chatRows.map((row) => row.agent_id)]);
      return {
        binding_id: binding._id, status: binding.status, transport: binding.transport,
        ordinary_private_chats: binding.private_chats ?? "owner",
        ordinary_shared_chats_open_to_members: chatRows.filter((row) =>
          ["group", "channel"].includes(row.kind) && (row.members === "everyone" || (!row.members && row.owner_seen))).length,
        org_guest_binding: binding.org_specialist?.guests ?? null,
        reachable_agents: reachable.map((id) => {
          const agent = agents.find((row) => row._id === id);
          return agent ? agentInventory(agent) : { agent_id: id, unavailable: true };
        }),
      };
    }),
    guest_skill_selection: bot.guest_skill_selection ?? null,
    external_routes: routes.filter((route) => route.channel_bot_id === bot._id
      && !bindings.some((binding) => binding.status === "active"
        && binding.route_id === route._id && binding.route_api_key_id === route.agent_api_key_id))
      .map((route) => externalRoute(bot, route)),
  };
});
print(EJSON.stringify({ generated_at: now, bots: report }, null, 2));
