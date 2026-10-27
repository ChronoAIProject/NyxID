import unittest
from cache_budget import plan


def cache(id, lane, ref='refs/heads/main', date='2026-10-27T00:00:00Z'):
    return dict(id=id, key=f'v0-rust-{lane}-Linux-x64-11111111-22222222', ref=ref,
                createdAt=date, lastAccessedAt=date, sizeInBytes=100)


class CacheBudgetTests(unittest.TestCase):
    def test_every_dist_cache_is_protected(self):
        rows=[cache(1,'dist-x86_64-unknown-linux-gnu'),cache(2,'dist-aarch64-apple-darwin')]
        rows[1]['key']=rows[1]['key'].replace('Linux-x64','macOS-arm64')
        self.assertEqual(plan(rows),[])

    def test_only_superseded_generations_are_deleted(self):
        old=cache(1,'cli-cli-test')
        new=cache(2,'cli-cli-test',date='2026-10-28T00:00:00Z')
        new['key']=new['key'].replace('11111111','aaaaaaaa')
        self.assertEqual([r['id'] for r in plan([old,new])],[1])

    def test_readers_need_main_writer_before_old_cache_is_removed(self):
        old=cache(1,'backend-billing-smoke-backend-billing-smoke')
        self.assertEqual(plan([old]),[])
        self.assertEqual([r['id'] for r in plan([old,cache(2,'backend-build')])],[1])

    def test_pr_never_replaces_main_or_another_live_lane(self):
        rows=[cache(1,'cli-cli-test'),cache(2,'cli-cli-test',ref='refs/pull/1/merge'),cache(3,'rust-clippy')]
        self.assertEqual([r['id'] for r in plan(rows)],[2])

    def test_unknown_and_npm_caches_untouched(self):
        rows=[cache(1,'future-build'),dict(cache(2,'cli'),key='node-cache-Linux-npm-hash')]
        self.assertEqual(plan(rows),[])

    def test_retired_docker_blobs_and_indexes_removed_across_refs(self):
        rows=[dict(cache(1,'x'),key='buildkit-blob-1-sha256:123'),
              dict(cache(2,'x',ref='refs/pull/2/merge'),key='index-machine-pr-1-123#1')]
        self.assertEqual([r['id'] for r in plan(rows)],[1,2])


if __name__=='__main__':
    unittest.main()
