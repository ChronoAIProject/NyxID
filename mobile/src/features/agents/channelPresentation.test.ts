import assert from "node:assert/strict";
import test from "node:test";
import { agentSchema } from "./agentContracts";
import { channelDetail } from "./channelPresentation";

test("gateway draining has a fixed message while relay draining keeps the platform slug", () => {
  const gatewayDetail = channelDetail({ id: "channel", platform: "discord", bot_label: "Bot", status: "active", gateway_native_activation: "gateway_native_draining" });
  assert.deepEqual(gatewayDetail, { kind: "message", value: "Discord intake is paused while the retained gateway release and safety drain are checked. NyxID keeps checking; retrying activation cannot bypass this safety check." });

  const relayDetail = channelDetail({ id: "channel", platform: "discord", bot_label: "Bot", status: "active", gateway_native_activation: "relay_draining" });
  assert.deepEqual(relayDetail, { kind: "platform", value: "discord" });
});

test("legacy channel summaries remain readable and unknown status codes stay additive", () => {
  const channel = { id: "channel", platform: "discord", bot_label: "Bot", status: "active" };
  assert.deepEqual(channelDetail(channel), { kind: "platform", value: "discord" });
  assert.deepEqual(channelDetail({ ...channel, gateway_native_activation: "future_code" }), { kind: "platform", value: "discord" });
  const schema = agentSchema.shape.channels;
  const legacy = schema.parse([channel])[0];
  const current = schema.parse([{ ...channel, gateway_native_activation: "gateway_native_draining" }])[0];
  assert.ok(legacy);
  assert.ok(current);
  assert.equal(legacy.gateway_native_activation, undefined);
  assert.equal(current.gateway_native_activation, "gateway_native_draining");
});
