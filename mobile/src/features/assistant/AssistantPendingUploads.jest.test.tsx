import { act, fireEvent, render, screen, within } from "@testing-library/react-native";
import { Platform, StyleSheet } from "react-native";
import { AssistantPendingUploads as PendingUploads } from "./AssistantPendingUploads";
import type { ComponentProps } from "react";
import type { PendingAssistantUpload } from "./AssistantUploadSession";

jest.mock("../../i18n/LanguageProvider", () => ({
  useAppLanguage: () => ({
    t: (phrase: string, values: Record<string, string> = {}) =>
      phrase.replace(/\{\{(\w+)\}\}/g, (_, key: string) => values[key] ?? key),
  }),
}));
jest.mock("../../theme/ThemeContext", () => ({
  useTheme: () => ({ colors: require("../../theme/mobileTheme").lightColors }),
}));
jest.mock("../../components/workspace/WorkspaceUI", () => {
  const { Text, Pressable } = require("react-native");
  return {
    Label: (props: object) => <Text {...props} />,
    ActionButton: ({
      label,
      disabled,
      onPress,
    }: {
      label: string;
      disabled: boolean;
      onPress: () => void;
    }) => (
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={label}
        accessibilityState={{ disabled }}
        disabled={disabled}
        onPress={onPress}
      >
        <Text>{label}</Text>
      </Pressable>
    ),
  };
});
jest.mock("expo-linear-gradient", () => {
  const { View } = require("react-native");
  return { LinearGradient: (props: object) => <View {...props} /> };
});
jest.mock("./AssistantPdfThumbnail", () => {
  const { View } = require("react-native");
  return {
    AssistantPdfThumbnail: (props: object) => (
      <View testID="pdf-thumbnail" {...props} />
    ),
  };
});
jest.mock("./AssistantImagePreview", () => {
  const { View } = require("react-native");
  return {
    AssistantImagePreview: (props: object) => (
      <View testID="image-viewer" {...props} />
    ),
  };
});
jest.mock("./AssistantUploadProgress", () => {
  const { View } = require("react-native");
  return {
    AssistantUploadProgress: (props: object) => (
      <View testID="upload-progress" {...props} />
    ),
  };
});

type UploadProps = ComponentProps<typeof PendingUploads>;
type RecoveryProps = "reselect" | "choose" | "dismiss" | "resume" | "canResume";

function AssistantPendingUploads(
  props: Omit<UploadProps, RecoveryProps> &
    Partial<Pick<UploadProps, RecoveryProps>>,
) {
  return (
    <PendingUploads
      reselect={false}
      choose={() => {}}
      dismiss={() => {}}
      resume={() => {}}
      canResume={false}
      {...props}
    />
  );
}

function file(
  key: number,
  name: string,
  type?: string,
): PendingAssistantUpload {
  return {
    key: String(key),
    handle: `file-${key}`,
    kind: "file",
    file: { name, uri: `file:///cache/${key}/${name}`, mimeType: type },
    progress: 100,
    previewReady: true,
    controller: new AbortController(),
    item: {
      id: `id-${key}`,
      label: name,
      content_type: type ?? "application/octet-stream",
      size: 3,
    },
  };
}
type Node = ReturnType<typeof screen.getByTestId>;
function descendants(node: Node): Node[] {
  return [
    node,
    ...node.children.flatMap((child) =>
      typeof child === "object" && child !== null ? descendants(child) : [],
    ),
  ];
}
const style = (node: { props: { style?: any } }) =>
  StyleSheet.flatten(node.props.style);
async function layout(width: number, height = 180) {
  await fireEvent(screen.getByTestId("composer-attachments"), "layout", {
    nativeEvent: { layout: { width, height } },
  });
}

for (const platform of ["ios", "android"] as const) {
  for (const count of [1, 2, 4, 10]) {
    it(`${platform}: ${count} attachments show about three equal squares in one constrained horizontal viewport at the actual composer width`, async () => {
      Platform.OS = platform;
      const files = Array.from({ length: count }, (_, i) =>
        file(i, `${i}.png`, "image/png"),
      );
      await render(
        <AssistantPendingUploads
          files={files}
          disabled={false}
          remove={jest.fn()}
        />,
      );
      for (const width of [260, 343.5, 734, 1176, 300]) {
        await layout(width);
        const cards = screen.getAllByTestId(/^composer-attachment-\d+$/);
        expect(cards).toHaveLength(count);
        const widths = cards.map((card) => style(card).width);
        expect(new Set(widths).size).toBe(1);
        expect(widths[0] * 3 + 24).toBeLessThanOrEqual(width);
        expect(widths[0] * 3 + 24).toBeGreaterThan(width - 3);
        expect(widths[0] * 4 + 32).toBeGreaterThan(width);
        cards.forEach((card) => expect(style(card).aspectRatio).toBe(1));
        const scroller = screen.getByTestId("composer-attachments");
        expect(scroller.props.horizontal).toBe(true);
        expect(scroller.props.contentContainerStyle.flexWrap).toBeUndefined();
        expect(style(scroller)).toMatchObject({
          width: "100%",
          maxWidth: "100%",
        });
        expect(style(scroller.parent!)).toMatchObject({
          width: "100%",
          minWidth: 0,
        });
        if (count <= 3) {
          expect(screen.queryByText(/^Attachments /)).toBeNull();
          expect(
            screen.queryByTestId("attachment-fade-left", {
              includeHiddenElements: true,
            }),
          ).toBeNull();
          expect(
            screen.queryByTestId("attachment-fade-right", {
              includeHiddenElements: true,
            }),
          ).toBeNull();
        } else {
          expect(screen.getByText(`Attachments 1–3 of ${count}`)).toBeTruthy();
        }
      }
    });
  }
}

it("mixed types, states, handles, decode failures and large text retain equal outer squares and independent circular remove targets", async () => {
  const files = [
    file(1, "a.png", "image/png"),
    file(2, "b.pdf", "application/pdf"),
    file(3, "c.txt", "text/plain"),
    file(
      4,
      "d.docx",
      "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    ),
  ];
  files[0]!.item = undefined;
  files[0]!.progress = 42;
  files[1]!.item = undefined;
  files[1]!.progress = 99;
  files[2]!.error =
    "A long upload error that must remain fully accessible when its visible line is truncated.";
  const remove = jest.fn();
  const view = await render(
    <AssistantPendingUploads files={files} disabled={false} remove={remove} />,
  );
  await layout(343);
  const before = screen.getAllByTestId(/^composer-attachment-\d+$/).map(style);
  expect(new Set(before.map((s) => s.width))).toEqual(new Set([106]));
  before.forEach((s) => expect(s.aspectRatio).toBe(1));
  expect(screen.getByText("TXT")).toBeTruthy();
  expect(screen.getByText("DOCX")).toBeTruthy();
  expect(screen.getByText("c.txt")).toBeTruthy();
  expect(screen.queryByText("a.png")).toBeNull();
  expect(screen.queryByText("b.pdf")).toBeNull();
  const failedCard = within(screen.getByTestId("composer-attachment-3"));
  const failureLabel = failedCard.getByText("Upload failed");
  expect(style(failureLabel).color).toBe(
    require("../../theme/mobileTheme").lightColors.danger,
  );
  expect(style(failureLabel.parent!)).toMatchObject({
    ...StyleSheet.absoluteFillObject,
    backgroundColor: "rgba(128, 128, 128, 0.48)",
  });
  expect(failedCard.queryByText(files[2]!.error!)).toBeNull();
  expect(screen.getByText(`c.txt: ${files[2]!.error}`)).toBeTruthy();
  expect(
    screen.getAllByTestId("upload-progress").map((n) => n.props.status.loader),
  ).toEqual(["determinate", "indeterminate", "none", "none"]);
  for (const row of files) {
    const button = screen.getByRole("button", {
      name: `Remove ${row.file.name}`,
    });
    expect(style(button)).toMatchObject({
      width: 44,
      height: 44,
      top: 0,
      right: 0,
      paddingTop: 4,
      paddingRight: 6,
      alignItems: "flex-end",
    });
    expect(style(button).backgroundColor).toBeUndefined();
    const circle = descendants(button).find((n) => style(n)?.width === 25.2)!;
    expect(style(circle)).toMatchObject({
      width: 25.2,
      height: 25.2,
      borderRadius: 12.6,
    });
    expect(style(circle).backgroundColor).toBeTruthy();
  }
  await fireEvent.press(screen.getByRole("button", { name: "Remove d.docx" }));
  expect(remove).toHaveBeenCalledWith("4");
  await fireEvent(screen.getByTestId("pdf-thumbnail"), "previewError");
  expect(screen.getByText("b.pdf")).toBeTruthy();
  expect(screen.getAllByTestId(/^composer-attachment-\d+$/).map(style)).toEqual(
    before,
  );
  await view.rerender(
    <AssistantPendingUploads files={files} disabled remove={remove} />,
  );
  await fireEvent.press(screen.getByRole("button", { name: "Remove d.docx" }));
  expect(remove).toHaveBeenCalledTimes(1);
});

it("photo taps open the full viewer, close preserves the draft, and removing/switching its row closes the viewer", async () => {
  const row = file(1, "a.png", "image/png");
  const remove = jest.fn();
  const view = await render(
    <AssistantPendingUploads files={[row]} disabled={false} remove={remove} />,
  );
  await layout(343);
  const photo = screen.getByRole("button", { name: "Preview a.png" });
  const body = descendants(screen.getByTestId("composer-attachment-1")).find(
    (n) => n.props.pointerEvents === "none",
  );
  expect(body).toBeTruthy();
  await fireEvent.press(photo);
  expect(screen.getByTestId("image-viewer").props.uri).toBe(row.file.uri);
  await act(() => screen.getByTestId("image-viewer").props.onClose());
  expect(screen.getByTestId("image-viewer").props.uri).toBeUndefined();
  expect(remove).not.toHaveBeenCalled();
  await fireEvent.press(photo);
  await view.rerender(
    <AssistantPendingUploads files={[]} disabled={false} remove={remove} />,
  );
  expect(screen.getByTestId("image-viewer").props.uri).toBeUndefined();
});

it("confirmed MIME wins over conflicting names, missing/generic MIME uses the suffix, failed previews show concise types", async () => {
  const files = [
    file(1, "photo.png", "application/pdf"),
    file(2, "report.pdf", "image/png"),
    file(3, "suffix.PNG"),
    file(4, "suffix.PDF"),
    file(5, "extensionless", "text/plain"),
    file(6, "unknown"),
  ];
  await render(
    <AssistantPendingUploads
      files={files}
      disabled={false}
      remove={jest.fn()}
    />,
  );
  expect(screen.getAllByTestId("pdf-thumbnail")).toHaveLength(2);
  expect(
    screen.getByRole("button", { name: "Preview report.pdf" }),
  ).toBeTruthy();
  expect(
    screen.getByRole("button", { name: "Preview suffix.PNG" }),
  ).toBeTruthy();
  expect(screen.queryByText("photo.png")).toBeNull();
  expect(screen.getByText("TXT")).toBeTruthy();
  expect(screen.getByText("Attachment")).toBeTruthy();
  const image = descendants(screen.getByTestId("composer-attachment-3")).find(
    (n) => n.props.source?.uri === files[2]!.file.uri,
  )!;
  await fireEvent(image, "error");
  expect(screen.getByText("suffix.PNG")).toBeTruthy();
  expect(screen.getByText("PNG")).toBeTruthy();
  await fireEvent(screen.getAllByTestId("pdf-thumbnail")[0]!, "previewError");
  expect(screen.getByText("photo.png")).toBeTruthy();
  expect(screen.getByText("PDF")).toBeTruthy();
});

it("unvalidated/rejected files never decode; a network error preserves a validated local preview and progress/status accessibility", async () => {
  const waiting = file(1, "waiting.png", "image/png");
  waiting.item = undefined;
  waiting.previewReady = undefined;
  waiting.progress = 0;
  const rejected = {
    ...file(2, "huge.pdf", "application/pdf"),
    previewReady: false,
    error: "Choose a file between 1 byte and 20 MB.",
  };
  const failed = {
    ...file(3, "network.png", "image/png"),
    error: "Upload failed. Please try again.",
  };
  await render(
    <AssistantPendingUploads
      files={[waiting, rejected, failed]}
      disabled={false}
      remove={jest.fn()}
    />,
  );
  expect(
    screen.queryByRole("button", { name: "Preview waiting.png" }),
  ).toBeNull();
  expect(screen.queryByTestId("pdf-thumbnail")).toBeNull();
  expect(screen.getByText("waiting.png")).toBeTruthy();
  expect(screen.getByText("huge.pdf")).toBeTruthy();
  expect(
    screen.getByRole("button", { name: "Preview network.png" }),
  ).toBeTruthy();
  expect(screen.getByText(`network.png: ${failed.error}`)).toBeTruthy();
  expect(screen.getAllByText("Upload failed")).toHaveLength(2);
});

it("keeps the attachment count on its own line and lists each failed file in a separate bullet, including offscreen files", async () => {
  const files = Array.from({ length: 6 }, (_, i) => file(i, `file-${i}.txt`, "text/plain"));
  const renderStrip = (rows: PendingAssistantUpload[]) => (
    <AssistantPendingUploads files={rows} disabled={false} remove={jest.fn()} />
  );
  const view = await render(renderStrip(files));
  await layout(343);
  const failed = {
    ...files[5]!,
    item: undefined,
    error: "Upload failed. Please try again.",
  };
  const withFailure = [...files.slice(0, 5), failed];
  await view.rerender(renderStrip(withFailure));
  const reason = "file-5.txt: Upload failed. Please try again.";
  expect(screen.getByText("Attachments 1–3 of 6").props.numberOfLines).toBe(1);
  expect(screen.getByText(reason)).toBeTruthy();

  await fireEvent.scroll(screen.getByTestId("composer-attachments"), {
    nativeEvent: { contentOffset: { x: 10000 } },
  });
  expect(screen.getByText("Attachments 4–6 of 6")).toBeTruthy();
  expect(screen.getByText(reason)).toBeTruthy();
  await view.rerender(renderStrip([
    ...withFailure.slice(0, 4),
    { ...files[4]!, item: undefined, error: "Upload cancelled." },
    failed,
  ]));
  const cancelled = "file-4.txt: Upload cancelled.";
  const cancelledRow = screen.getByText(cancelled).parent!;
  const failedRow = screen.getByText(reason).parent!;
  expect(failedRow).not.toBe(cancelledRow);
  for (const row of [cancelledRow, failedRow]) {
    expect(within(row).getByText("•", { includeHiddenElements: true })).toBeTruthy();
  }

  await view.rerender(renderStrip([failed]));
  expect(screen.getByText("Attachments (1)")).toBeTruthy();
  expect(screen.queryByText(cancelled)).toBeNull();
  expect(screen.getByText(reason)).toBeTruthy();
  await view.rerender(renderStrip(files.slice(0, 5)));
  expect(screen.getByText("Attachments 1–3 of 5")).toBeTruthy();
  expect(screen.queryByText(/failed to upload|Upload failed/)).toBeNull();
});

it("horizontal swipes, resize, removal and additions update/clamp the range and directional fades; fourth and tenth remove controls work", async () => {
  let files = Array.from({ length: 10 }, (_, i) =>
    file(i, `${i}.png`, "image/png"),
  );
  const remove = jest.fn((key: string) => {
    files = files.filter((row) => row.key !== key);
  });
  const view = await render(
    <AssistantPendingUploads files={files} disabled={false} remove={remove} />,
  );
  await layout(343.5, 114);
  const scroll = () => screen.getByTestId("composer-attachments");
  const left = () =>
    screen.queryByTestId("attachment-fade-left", {
      includeHiddenElements: true,
    });
  const right = () =>
    screen.queryByTestId("attachment-fade-right", {
      includeHiddenElements: true,
    });
  expect(scroll().props.bounces).toBe(true);
  expect(scroll().props.overScrollMode).toBe("always");
  expect(scroll().props.keyboardShouldPersistTaps).toBe("always");
  expect(left()).toBeNull();
  expect(style(right()!).width).toBe(32);
  expect(screen.getByText("Attachments 1–3 of 10")).toBeTruthy();
  await fireEvent.scroll(scroll(), {
    nativeEvent: { contentOffset: { x: 114 } },
  });
  expect(screen.getByText("Attachments 2–4 of 10")).toBeTruthy();
  expect(left()).not.toBeNull();
  expect(right()).not.toBeNull();
  await fireEvent.press(screen.getByRole("button", { name: "Remove 3.png" }));
  expect(remove).toHaveBeenLastCalledWith("3");
  await view.rerender(
    <AssistantPendingUploads files={files} disabled={false} remove={remove} />,
  );
  expect(screen.getByText("Attachments 2–4 of 9")).toBeTruthy();
  await fireEvent.scroll(scroll(), {
    nativeEvent: { contentOffset: { x: 10000 } },
  });
  expect(screen.getByText("Attachments 7–9 of 9")).toBeTruthy();
  expect(right()).toBeNull();
  expect(left()).not.toBeNull();
  await fireEvent.press(screen.getByRole("button", { name: "Remove 9.png" }));
  expect(remove).toHaveBeenLastCalledWith("9");
  files = files.slice(0, 4);
  await view.rerender(
    <AssistantPendingUploads files={files} disabled={false} remove={remove} />,
  );
  expect(screen.getByText("Attachments 2–4 of 4")).toBeTruthy();
  await layout(734, 244);
  expect(style(scroll()).height).toBe(244);
  expect(screen.getByText(/^Attachments .* of 4$/)).toBeTruthy();
  files = files.slice(0, 3);
  await view.rerender(
    <AssistantPendingUploads files={files} disabled={false} remove={remove} />,
  );
  expect(screen.queryByText(/^Attachments /)).toBeNull();
  expect(left()).toBeNull();
  expect(right()).toBeNull();
  files = [...files, file(12, "new.pdf", "application/pdf")];
  await view.rerender(
    <AssistantPendingUploads files={files} disabled={false} remove={remove} />,
  );
  expect(screen.getByText("Attachments 1–3 of 4")).toBeTruthy();
  expect(right()).not.toBeNull();
  const range = screen.getByText("Attachments 1–3 of 4");
  // The overlay belongs to the viewport; the range is a sibling outside it.
  expect(right()!.parent!.parent).not.toBe(range.parent);
});

it("restored drafts offer explicit recovery alongside a scrollable attachment strip", async () => {
  const files = [file(1, "finished.png", "image/png"), file(2, "draft.txt")];
  files[1]!.item = undefined;
  files[1]!.resumable = true;
  const choose = jest.fn();
  const dismiss = jest.fn();
  const resume = jest.fn();
  const remove = jest.fn();
  const view = await render(
    <AssistantPendingUploads
      files={files}
      disabled={false}
      remove={remove}
      reselect
      choose={choose}
      dismiss={dismiss}
      resume={resume}
      canResume={false}
    />,
  );
  expect(screen.getByRole("button", { name: "Choose files" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Resume attachment uploads" })).toBeDisabled();
  expect(screen.getByTestId("composer-attachments").props.horizontal).toBe(true);
  expect(screen.getByText("Ready to resume")).toBeTruthy();
  await view.rerender(
    <AssistantPendingUploads
      files={files}
      disabled={false}
      remove={remove}
      reselect
      choose={choose}
      dismiss={dismiss}
      resume={resume}
      canResume
    />,
  );
  await fireEvent.press(screen.getByRole("button", { name: "Choose files" }));
  await fireEvent.press(screen.getByRole("button", { name: "Resume attachment uploads" }));
  await fireEvent.press(screen.getByRole("button", { name: "Dismiss" }));
  await fireEvent.press(screen.getByRole("button", { name: "Remove draft.txt" }));
  expect(choose).toHaveBeenCalledTimes(1);
  expect(resume).toHaveBeenCalledTimes(1);
  expect(dismiss).toHaveBeenCalledTimes(1);
  expect(remove).toHaveBeenCalledWith("2");
});
