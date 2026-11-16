import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { hookHarness } from "./assistantHookTestHarness";
import type * as Cache from "./attachmentCache";

test("attachment cache recovery removes crash orphans only and never deletes a directory owned by a live operation", () => {
  const deleted: string[] = [];
  let listings = 0;
  let entries: (Directory | { name: string; uri: string })[] = [];
  class Directory {
    name: string;
    uri: string;
    constructor(parent: unknown, name = "") {
      this.name = name;
      this.uri = `${parent}/${name}`;
    }
    list() {
      listings++;
      return entries;
    }
    delete() {
      deleted.push(this.name);
    }
  }
  const h = hookHarness<void>({
    "expo-file-system": {
      Directory,
      Paths: { cache: "file:///private/cache" },
    },
  });
  const cache = h.load(
    fileURLToPath(new URL("./attachmentCache.ts", import.meta.url)),
  ) as typeof Cache;
  const orphan = new Directory("cache", "nyx-attachment-orphan");
  const live = new Directory("cache", "nyx-attachment-live");
  const unrelated = new Directory("cache", "other-cache");
  const plainFile = {
    name: "nyx-attachment-file",
    uri: "cache/nyx-attachment-file",
  };
  entries = [orphan, live, unrelated, plainFile];
  const release = cache.ownAttachmentCache(live);
  cache.sweepAttachmentCaches();
  assert.deepEqual(deleted, [orphan.name]);
  entries = [live, unrelated, plainFile];
  cache.sweepAttachmentCaches();
  assert.deepEqual(
    deleted,
    [orphan.name],
    "another attachment mount preserves the live share or preview",
  );
  release();
  cache.sweepAttachmentCaches();
  assert.equal(listings, 1, "mounts and refocuses never rescan this process");
  assert.deepEqual(deleted, [orphan.name]);
  // A fresh module models the next process recovering the now-orphaned directory.
  const nextProcess = h.load(
    fileURLToPath(new URL("./attachmentCache.ts", import.meta.url)),
  ) as typeof Cache;
  nextProcess.sweepAttachmentCaches();
  assert.equal(listings, 2);
  assert.deepEqual(deleted, [orphan.name, live.name]);
});

test("an unavailable attachment cache is attempted only once per process", () => {
  let listings = 0;
  class Directory {
    list() {
      listings++;
      throw new Error("cache unavailable");
    }
  }
  const h = hookHarness<void>({
    "expo-file-system": { Directory, Paths: { cache: "cache" } },
  });
  const cache = h.load(
    fileURLToPath(new URL("./attachmentCache.ts", import.meta.url)),
  ) as typeof Cache;
  cache.sweepAttachmentCaches();
  cache.sweepAttachmentCaches();
  assert.equal(listings, 1);
});
