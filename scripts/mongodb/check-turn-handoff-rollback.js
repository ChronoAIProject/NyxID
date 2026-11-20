db.getMongo().setReadPref("primary");
if (db.getCollectionInfos({ name: "assistant_conversations" }).length !== 1) {
  throw new Error("Select the NyxID database containing assistant_conversations.");
}

const present = (path) => ({ $ne: [{ $ifNull: [path, null] }, null] });
const counts = db.assistant_conversations.aggregate([
  { $match: { $or: [
    { "active_turn.restart_admission": { $ne: null } },
    { "active_turn.restart_offer": { $ne: null } },
    { stopped_window: { $ne: null } },
  ] } },
  { $group: {
    _id: { lane: { $ifNull: ["$active_turn.restart_admission.team_source.kind", "other"] }, state: { $switch: {
      branches: [
        { case: present("$stopped_window"), then: "cancel_unconfirmed" },
        { case: present("$active_turn.restart_offer.claimed_by"), then: "claimed" },
        { case: present("$active_turn.restart_offer"), then: "offered" },
      ],
      default: "subscribed",
    } } },
    count: { $sum: 1 },
  } },
  { $sort: { _id: 1 } },
], { readConcern: { level: "majority" }, maxTimeMS: 10000 }).toArray();
printjson(counts);
quit(counts.length === 0 ? 0 : 2);
