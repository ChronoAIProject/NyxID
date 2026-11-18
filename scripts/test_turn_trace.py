"""Regression tests for the offline, metadata-only turn trace reader."""
import importlib.util
import io
import json
from pathlib import Path
import unittest
from unittest.mock import patch
import uuid

spec = importlib.util.spec_from_file_location("turn_trace", Path(__file__).with_name("turn-trace.py"))
trace = importlib.util.module_from_spec(spec)
spec.loader.exec_module(trace)
TURN = str(uuid.uuid4())
INSTANCE = str(uuid.uuid4())
REQUEST = str(uuid.uuid4())


def record(**changes):
    value = dict(schema_version=1, target=trace.TARGET, service="nyxid", instance_id=INSTANCE,
                 timestamp="2026-11-07T01:00:00Z", turn_id=TURN, trace_id=TURN.replace("-", ""),
                 span_id="0000000000000001", request_id=REQUEST, stage="engine", phase="start")
    value.update(changes)
    return value


def read(values):
    return trace.read_records([io.BytesIO(b"\n".join(json.dumps(v).encode() for v in values))], TURN)


class ReaderTests(unittest.TestCase):
    def test_identifiers(self):
        self.assertEqual(trace.identifier(TURN.replace("-", "").upper()), TURN)
        for value in ["needle", str(uuid.uuid1()), None, [], {}]:
            self.assertIsNone(trace.identifier(value))

    def test_malformed_fields_never_echo_or_crash(self):
        for field in ["service", "stage", "phase", "outcome", "timestamp", "span_id", "turn_id", "request_id"]:
            for value in [[], {}, "secret-canary", True]:
                self.assertIsNone(trace.clean_record(record(**{field: value})), (field, value))
        for field in ["duration_ms", "http_status", "error_code"]:
            self.assertIsNone(trace.clean_record(record(**{field: True})))

    def test_unknown_fields_are_discarded(self):
        rows, invalid, _ = read([record(body="secret-canary", headers={"Authorization": "secret-canary"}), record(phase="end", outcome="failed", duration_ms=10)])
        result = trace.report(TURN, rows, invalid)
        self.assertNotIn("secret-canary", json.dumps(result))
        self.assertNotIn("secret-canary", trace.render_text(result))
        self.assertEqual(result["coverage"]["services_missing"], ["nyxagent"])

    def test_out_of_order_duplicates_and_instance_recovery(self):
        start = record(stage="admission")
        end = record(stage="execution", phase="milestone", outcome="completed", duration_ms=100, instance_id=str(uuid.uuid4()))
        rows, invalid, truncated = read([end, start, start])
        result = trace.report(TURN, rows, invalid, truncated)
        self.assertEqual(len(rows), 2)
        self.assertEqual(result["execution"]["duration_ms"], 100)
        self.assertFalse(result["execution"]["incomplete"])
        self.assertEqual(len(result["execution"]["instances"]), 2)

    def test_queued_ingress_can_precede_or_follow_link(self):
        ingress = record(stage="ingress", turn_id=None, trace_id=None, span_id="0000000000000009")
        link = record(stage="admission", phase="link", ingress_request_id=REQUEST)
        for values in [[ingress, link], [link, ingress]]:
            rows, _, _ = read(values)
            result = trace.report(TURN, rows)
            self.assertEqual(len(rows), 2)
            self.assertEqual(result["coverage"]["ingress_requests_missing"], [])
        result = trace.report(TURN, [trace.clean_record(link)])
        self.assertEqual(result["coverage"]["ingress_requests_missing"], [REQUEST])

    def test_clock_skew_is_not_used_as_duration(self):
        rows, _, _ = read([record(timestamp="2026-11-07T01:00:01Z"), record(phase="end", outcome="completed", duration_ms=5)])
        result = trace.report(TURN, rows)
        self.assertEqual(result["steps"][0]["duration_ms"], 5)

    def test_incomplete_and_unknown_are_partial(self):
        for values in [[record()], [record(), record(phase="end", outcome="unknown", duration_ms=0)]]:
            rows, _, _ = read(values)
            self.assertEqual(trace.report(TURN, rows)["coverage"]["incomplete_spans"], 1)

    def test_streamed_milestone_is_visible_without_confirmation(self):
        rows, _, _ = read([record(stage="channel", phase="milestone", outcome="streamed")])
        result = trace.report(TURN, rows)
        self.assertEqual(result["coverage"]["services_missing"], ["cma"])
        self.assertEqual(result["delivery"][0]["outcome"], "streamed")
        self.assertIn("channel milestone: streamed", trace.render_text(result))
        self.assertIn("no confirmation observed", trace.render_text(result))

    def test_one_confirmed_part_does_not_imply_complete_delivery(self):
        rows, _, _ = read([record(stage="channel_part", part_index=0), record(stage="channel_part", phase="end", outcome="confirmed", part_index=0)])
        result = trace.report(TURN, rows)
        self.assertIn("no confirmation observed for a complete send", trace.render_text(result))

    def test_gateway_transport_is_not_platform_confirmation(self):
        rows, _, _ = read([record(stage="gateway"), record(stage="gateway", phase="end", outcome="completed", http_status=200, duration_ms=10)])
        result = trace.report(TURN, rows)
        self.assertEqual(result["coverage"]["services_missing"], ["cma"])
        self.assertEqual(result["delivery"], [])
        self.assertIn("no confirmation observed", trace.render_text(result))

    def test_stream_and_record_limits(self):
        with patch.object(trace, "MAX_RECORDS", 1):
            rows, _, truncated = read([record(), record(phase="end")])
            self.assertEqual(len(rows), 1)
            self.assertTrue(truncated)
        with patch.object(trace, "MAX_BYTES", 4096):
            _, invalid, truncated = trace.read_records([io.BytesIO(b"x" * 10000)], TURN)
            self.assertEqual(invalid, 1)
            self.assertTrue(truncated)

    def test_unrelated_turns_and_invalid_json(self):
        stream = io.BytesIO(b"secret-canary\n" + json.dumps(record(turn_id=str(uuid.uuid4()))).encode())
        rows, invalid, _ = trace.read_records([stream], TURN)
        self.assertEqual(rows, [])
        self.assertEqual(invalid, 2)

    def test_cycle_is_bounded(self):
        rows, _, _ = read([record(parent_span_id="0000000000000001")])
        self.assertLess(len(trace.render_text(trace.report(TURN, rows))), 2000)


if __name__ == "__main__":
    unittest.main()
