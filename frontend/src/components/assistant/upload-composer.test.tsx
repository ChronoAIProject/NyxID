import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { UploadComposer } from "./upload-composer";
import {
  createUploadDraft,
  removeUpload,
  uploadFile,
} from "@/lib/assistant/uploads";
import { useAssistantDraftStore } from "@/stores/assistant-draft-store";

vi.mock("@/lib/assistant/uploads", () => ({
  UPLOAD_ACCEPT: ".png,.pdf,.txt",
  createUploadDraft: vi.fn().mockResolvedValue("nyxa-draft"),
  removeUpload: vi.fn().mockResolvedValue(undefined),
  uploadFile: vi.fn(),
}));
const base = {
  active: false,
  sending: false,
  ownerUserId: "owner",
  draftKey: "screen:nyxagent:assistant",
  onStop: vi.fn(),
  onSend: vi.fn().mockResolvedValue(undefined),
};
const item = {
  id: "attachment-1",
  label: "notes.txt",
  content_type: "text/plain" as const,
  size: 5,
  origin: "user_upload" as const,
};
function choose(file = new File(["notes"], "notes.txt")) {
  fireEvent.change(screen.getByLabelText("Choose attachments"), {
    target: { files: [file] },
  });
}
beforeEach(() => {
  vi.clearAllMocks();
  useAssistantDraftStore.setState({ ownerUserId: null, drafts: {} });
  vi.mocked(uploadFile).mockResolvedValue(item);
});
describe("Assistant uploads composer", () => {
  it("creates a draft once, shows progress and sends attachment-only messages", async () => {
    let resolve!: (value: typeof item) => void;
    vi.mocked(uploadFile).mockImplementation(
      (_scope, _file, _signal, progress) => {
        progress(42);
        return new Promise((done) => {
          resolve = done;
        });
      },
    );
    render(
      <UploadComposer
        {...base}
        scope={{ kind: "conversations", agentId: "specialist" }}
      />,
    );
    choose();
    expect(await screen.findByText("Uploading 42%")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Send message" })).toBeDisabled();
    await act(async () => resolve(item));
    expect(screen.getByRole("button", { name: "Send message" })).toBeEnabled();
    fireEvent.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() =>
      expect(base.onSend).toHaveBeenCalledWith("", {
        attachmentIds: [item.id],
        conversationId: "nyxa-draft",
      }),
    );
    expect(createUploadDraft).toHaveBeenCalledWith("specialist");
    expect(removeUpload).not.toHaveBeenCalled();
  });
  it("queues a ten-file selection so extraction admission is not flooded", async () => {
    const completions: Array<() => void> = [];
    vi.mocked(uploadFile).mockImplementation(
      (_scope, file) =>
        new Promise((resolve) => {
          completions.push(() =>
            resolve({ ...item, id: file.name, label: file.name }),
          );
        }),
    );
    render(
      <UploadComposer {...base} scope={{ kind: "conversations", id: "one" }} />,
    );
    fireEvent.change(screen.getByLabelText("Choose attachments"), {
      target: {
        files: Array.from(
          { length: 10 },
          (_, i) => new File(["notes"], `${i}.txt`),
        ),
      },
    });
    for (let i = 0; i < 10; i++) {
      await waitFor(() => expect(uploadFile).toHaveBeenCalledTimes(i + 1));
      expect(
        screen.getByRole("button", { name: "Send message" }),
      ).toBeDisabled();
      await act(async () => completions[i]!());
    }
    expect(screen.getByRole("button", { name: "Send message" })).toBeEnabled();
    fireEvent.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() =>
      expect(base.onSend).toHaveBeenCalledWith("", {
        attachmentIds: Array.from({ length: 10 }, (_, i) => `${i}.txt`),
        conversationId: "one",
      }),
    );
  });
  it("keeps per-file errors visible and prevents send until removed", async () => {
    vi.mocked(uploadFile).mockRejectedValue(
      new Error("Encrypted PDF. Export an unencrypted copy."),
    );
    render(
      <UploadComposer {...base} scope={{ kind: "conversations", id: "one" }} />,
    );
    choose(new File(["encrypted"], "private.pdf"));
    expect(await screen.findByRole("alert")).toHaveTextContent("Encrypted PDF");
    fireEvent.change(screen.getByRole("textbox"), {
      target: { value: "hello" },
    });
    expect(screen.getByRole("button", { name: "Send message" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Remove private.pdf" }));
    expect(screen.getByRole("button", { name: "Send message" })).toBeEnabled();
  });
  it("supports paste and drop, deletes an unsent upload and scopes group files", async () => {
    render(
      <UploadComposer {...base} scope={{ kind: "groups", id: "group-one" }} />,
    );
    fireEvent.paste(screen.getByRole("textbox"), {
      clipboardData: { files: [new File(["notes"], "notes.txt")] },
    });
    await screen.findByText("Ready");
    fireEvent.click(screen.getByRole("button", { name: "Remove notes.txt" }));
    expect(removeUpload).toHaveBeenCalledWith(
      { kind: "groups", id: "group-one" },
      item.id,
    );
    fireEvent.drop(screen.getByRole("textbox"), {
      dataTransfer: { files: [new File(["notes"], "notes.txt")] },
    });
    await screen.findByText("Ready");
    expect(uploadFile).toHaveBeenCalledTimes(2);
    expect(createUploadDraft).not.toHaveBeenCalled();
  });
  it("aborts pending uploads on conversation change without attaching them to the next", async () => {
    vi.mocked(uploadFile).mockImplementation(() => new Promise(() => {}));
    const { rerender } = render(
      <UploadComposer
        key="one"
        {...base}
        scope={{ kind: "conversations", id: "one" }}
      />,
    );
    choose();
    await waitFor(() => expect(uploadFile).toHaveBeenCalled());
    const signal = vi.mocked(uploadFile).mock.calls[0]![2];
    rerender(
      <UploadComposer
        key="two"
        {...base}
        draftKey="conv:two"
        scope={{ kind: "conversations", id: "two" }}
      />,
    );
    expect(signal.aborted).toBe(true);
    expect(screen.queryByText("notes.txt")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Send message" })).toBeDisabled();
  });
});
