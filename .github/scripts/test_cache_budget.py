import unittest
from datetime import datetime, timezone
from io import StringIO
from contextlib import redirect_stdout

from cache_budget import plan, report

NOW = datetime(2026, 10, 28, tzinfo=timezone.utc)


def cache(id, lane, ref='refs/heads/main', date='2026-10-27T00:00:00Z', size=100,
          os='Linux', arch='x64'):
    return dict(id=id, key=f'v0-rust-{lane}-{os}-{arch}-11111111-22222222', ref=ref,
                date=date, createdAt=date, lastAccessedAt=date, sizeInBytes=size)


def incremental(id, lane, ref='refs/heads/main', date='2026-10-27T00:00:00Z',
                os='Linux', arch='x64', generation='a' * 64, sha='b' * 40):
    return dict(
        id=id,
        key=f'v0-rust-incremental-{lane}-{os}-{arch}-{generation}-{sha}',
        ref=ref,
        createdAt=date,
        lastAccessedAt=date,
        sizeInBytes=100,
    )


def dist(id, target, ref='refs/heads/main', date='2026-10-27T00:00:00Z'):
    return dict(id=id, key=f'v0-rust-dist-{target}-Linux-x64-11111111-22222222', ref=ref,
                createdAt=date, lastAccessedAt=date, sizeInBytes=100)


def node(id, manager='npm', lock='a', ref='refs/heads/main', date='2026-10-27T00:00:00Z'):
    return dict(id=id, key=f'node-cache-Linux-x64-{manager}-{lock}', ref=ref,
                createdAt=date, lastAccessedAt=date, sizeInBytes=100)


def coverage(id, sha, ref='refs/heads/main', date='2026-10-27T00:00:00Z'):
    return dict(id=id, key=f'coverage-backend-base-v2-{sha}-{"f" * 64}', ref=ref,
                createdAt=date, lastAccessedAt=date, sizeInBytes=100)


class CacheBudgetTests(unittest.TestCase):
    def test_dist_keeps_newest_per_target_and_prunes_non_main(self):
        rows = [
            dist(1, 'x86_64-unknown-linux-gnu', date='2026-10-20T00:00:00Z'),
            dist(2, 'x86_64-unknown-linux-gnu', date='2026-10-27T00:00:00Z'),
            dist(3, 'aarch64-unknown-linux-gnu'),
            dist(4, 'x86_64-unknown-linux-gnu', ref='refs/heads/refs/tags/v1'),
        ]
        self.assertEqual([row['id'] for row in plan(rows, NOW)], [1, 4])

    def test_dist_keeps_newest_main_but_removes_non_main_copies(self):
        rows = [dist(1, 'x86_64-unknown-linux-gnu'),
                dist(2, 'aarch64-unknown-linux-gnu', ref='refs/pull/1/merge')]
        self.assertEqual([row['id'] for row in plan(rows, NOW)], [2])

    def test_node_keeps_each_main_lockfile_and_removes_only_pr_duplicates(self):
        rows = [
            node(1, lock='a' * 64), node(2, lock='b' * 64, date='2026-10-28T00:00:00Z'),
            node(3, manager='pnpm', lock='c' * 64),
            node(4, lock='a' * 64, ref='refs/pull/1/merge'),
            node(5, lock='b' * 64, ref='refs/pull/2/merge'),
            node(6, manager='pnpm', lock='c' * 64, ref='refs/pull/3/merge'),
            node(7, lock='d' * 64, ref='refs/pull/4/merge'),
        ]
        self.assertEqual([row['id'] for row in plan(rows, NOW)], [4, 5, 6])
        self.assertEqual(plan([node(7, lock='d' * 64, ref='refs/pull/4/merge')], NOW), [])

    def test_coverage_keeps_newest_30_union_recent_window_and_prunes_non_main(self):
        sha = 'a' * 40
        rows = [coverage(i, sha, date='2026-09-01T00:00:00Z') for i in range(1, 36)]
        rows += [coverage(100 + i, sha, date='2026-10-20T00:00:00Z') for i in range(35)]
        rows += [coverage(200, sha, ref='refs/heads/refs/tags/v1')]
        removed = {row['id'] for row in plan(rows, NOW)}
        # The newest 30 are recent, but the 14-day union retains all 35 recent
        # rows; only the 35 old rows and the non-main copy are removed.
        self.assertEqual(len(removed), 36)
        self.assertIn(1, removed)
        self.assertNotIn(134, removed)
        self.assertIn(200, removed)

    def test_only_superseded_rust_generations_are_deleted(self):
        old = cache(1, 'cli-cli-test')
        new = cache(2, 'cli-cli-test', date='2026-10-28T00:00:00Z')
        self.assertEqual([row['id'] for row in plan([old, new], NOW)], [1])

    def test_incremental_keeps_newest_per_lane_and_os_arch(self):
        rows = [
            incremental(1, 'backend-build'),
            incremental(2, 'backend-build', date='2026-10-28T00:00:00Z'),
            incremental(3, 'coverage-backend-build'),
            incremental(4, 'backend-build', os='Darwin', arch='arm64'),
            incremental(5, 'backend-build', ref='refs/pull/1/merge'),
        ]
        self.assertEqual([row['id'] for row in plan(rows, NOW)], [1, 5])

    def test_incremental_keeps_only_main_entry_when_no_replacement_exists(self):
        rows = [incremental(1, 'backend-build', ref='refs/pull/1/merge')]
        self.assertEqual(plan(rows, NOW), [])

    def test_rust_lane_keeps_os_arch_families_independently(self):
        rows = [
            cache(1, 'cli-cli-test', os='Linux', arch='x64'),
            cache(2, 'cli-cli-test', os='Linux', arch='x64', date='2026-10-28T00:00:00Z'),
            cache(3, 'cli-cli-test', os='Darwin', arch='arm64'),
        ]
        self.assertEqual([row['id'] for row in plan(rows, NOW)], [1])

    def test_retired_rust_lane_waits_for_replacement(self):
        old = cache(1, 'backend-billing-smoke-backend-billing-smoke')
        self.assertEqual(plan([old], NOW), [])
        self.assertEqual([row['id'] for row in plan([old, cache(2, 'backend-build')], NOW)], [1])

    def test_pr_never_replaces_main_or_another_live_lane(self):
        rows = [cache(1, 'cli-cli-test'), cache(2, 'cli-cli-test', ref='refs/pull/1/merge'), cache(3, 'rust-clippy')]
        self.assertEqual([row['id'] for row in plan(rows, NOW)], [2])

    def test_unknown_families_are_untouched(self):
        rows = [cache(1, 'future-build'), dict(cache(2, 'cli'), key='future-cache-v2')]
        self.assertEqual(plan(rows, NOW), [])

    def test_retired_docker_blobs_and_indexes_removed(self):
        rows = [dict(cache(1, 'x'), key='buildkit-blob-1-sha256:123'),
                dict(cache(2, 'x', ref='refs/pull/2/merge'), key='index-machine-pr-1-123#1')]
        self.assertEqual([row['id'] for row in plan(rows, NOW)], [1, 2])

    def test_soft_cap_warns_without_changing_removals(self):
        rows = [cache(1, 'cli-cli-test', size=500), cache(2, 'future', size=500)]
        expected = plan(rows, NOW)
        output = StringIO()
        with redirect_stdout(output):
            report(rows, expected, 0.0000001)
        self.assertIn('::warning::Retained Actions cache total', output.getvalue())
        self.assertEqual(plan(rows, NOW), expected)

    def test_malformed_dates_are_retained_conservatively(self):
        rows = [coverage(i, 'a' * 40, date=f'2026-09-01T00:00:00Z') for i in range(1, 35)]
        rows.append(coverage(100, 'a' * 40, date='not-a-date'))
        self.assertNotIn(100, {row['id'] for row in plan(rows, NOW)})


if __name__ == '__main__':
    unittest.main()
