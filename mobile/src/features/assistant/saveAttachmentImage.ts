import { Platform, Share } from "react-native";
import { Directory, type File } from "expo-file-system";
import type { TranslationValues } from "../../i18n/translate";

export type ImageSaveResult = { phrase: string; values?: TranslationValues };
export const IMAGE_NOT_SAVED = "Image not saved.";
export const IMAGE_SAVE_FAILED = "Could not save the image. Please try again.";

export function imageShareResult(result: {
  action: string;
  activityType?: string | null;
}): ImageSaveResult {
  if (result.action === Share.dismissedAction)
    return { phrase: IMAGE_NOT_SAVED };
  if (result.activityType === "com.apple.UIKit.activity.SaveToCameraRoll")
    return { phrase: "Saved to Photos." };
  if (
    result.activityType &&
    /(?:SaveToFiles|SaveToDocuments)$/.test(result.activityType)
  )
    return { phrase: "Saved to Files." };
  if (result.activityType === "com.apple.UIKit.activity.CopyToPasteboard")
    return { phrase: "Image copied." };
  return { phrase: "Shared." };
}

export async function saveAttachmentImage({
  file,
  bytes,
  name,
  contentType,
  anchor,
  alive,
}: {
  file?: Pick<File, "uri">;
  bytes: Uint8Array;
  name: string;
  contentType: string;
  anchor: number | null;
  alive: () => boolean;
}): Promise<ImageSaveResult> {
  if (!alive()) return { phrase: IMAGE_NOT_SAVED };
  if (Platform.OS === "ios") {
    if (!file?.uri.startsWith("file://")) throw new Error(IMAGE_SAVE_FAILED);
    if (anchor === null) throw new Error(IMAGE_SAVE_FAILED);
    return imageShareResult(await Share.share({ url: file.uri }, { anchor }));
  }
  if (Platform.OS !== "android") throw new Error(IMAGE_SAVE_FAILED);
  let folder: Directory;
  try {
    folder = await Directory.pickDirectoryAsync();
  } catch (error) {
    const code = (error as { code?: string } | null)?.code;
    if (
      code === "ERR_CANCELED" ||
      code === "ERR_PICKER_CANCELLED" ||
      code === "ERR_DIRECTORY_PICKER_CANCELED"
    )
      return { phrase: IMAGE_NOT_SAVED };
    throw new Error(IMAGE_SAVE_FAILED);
  }
  if (!alive()) return { phrase: IMAGE_NOT_SAVED };
  const destination = folder.createFile(name, contentType);
  try {
    destination.write(bytes);
  } catch {
    // A failed write must not leave an empty or partial file in the chosen folder.
    try {
      destination.delete();
    } catch {
      /* The provider may have removed it. */
    }
    throw new Error(IMAGE_SAVE_FAILED);
  }
  const folderName = folder.name?.trim();
  return folderName
    ? { phrase: "Saved to {{folder}}.", values: { folder: folderName } }
    : { phrase: "Saved to the selected folder." };
}
