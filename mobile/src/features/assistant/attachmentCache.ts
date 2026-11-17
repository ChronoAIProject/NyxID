import { Directory, Paths } from "expo-file-system";

const liveDirectories = new Set<string>();
let swept = false;

export function ownAttachmentCache(directory: Pick<Directory, "uri">) {
  liveDirectories.add(directory.uri);
  return () => liveDirectories.delete(directory.uri);
}

/** Mount-time recovery for cache files left by a crash or process termination. */
export function sweepAttachmentCaches() {
  if (swept) return;
  swept = true;
  try {
    for (const entry of new Directory(Paths.cache).list()) {
      if (
        entry instanceof Directory &&
        entry.name.startsWith("nyx-attachment-") &&
        !liveDirectories.has(entry.uri)
      ) {
        try {
          entry.delete();
        } catch {
          // Cache eviction or another mount may already have removed it.
        }
      }
    }
  } catch {
    // An unavailable cache must not prevent opening an attachment.
  }
}
