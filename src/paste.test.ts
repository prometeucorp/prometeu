import { expect, it, vi, type Mock } from "vitest";
import { invoke } from "./ipc";
// `vi.mocked(invoke)` makes the checker relate the whole IPC registry to the mock type, which costs minutes.
const invokeMock = invoke as unknown as Mock;
import { pasteFiles } from "./paste";

vi.mock("./ipc", () => ({ invoke: vi.fn() }));

const event = (files: unknown[], types = files.length ? ["Files"] : ["text/plain"]) =>
  ({ clipboardData: { files, types }, preventDefault: vi.fn() }) as unknown as ClipboardEvent & { preventDefault: ReturnType<typeof vi.fn> };

it("attaches materialized paths and keeps the default paste for text", async () => {
  invokeMock.mockResolvedValue(["/private/attachments/pasted.png"]);
  const put = vi.fn();
  const fail = vi.fn();

  const text = event([]);
  pasteFiles(text, put, fail);
  expect(text.preventDefault).not.toHaveBeenCalled();
  expect(invoke).not.toHaveBeenCalled();

  const image = event([{}]);
  pasteFiles(image, put, fail);
  expect(image.preventDefault).toHaveBeenCalled();
  await vi.waitFor(() => expect(put).toHaveBeenCalledWith(["/private/attachments/pasted.png"]));
  expect(fail).not.toHaveBeenCalled();
});

it("reports backend failures instead of attaching nothing silently", async () => {
  invokeMock.mockRejectedValue("chat.drop.failed");
  const put = vi.fn();
  const fail = vi.fn();

  pasteFiles(event([{}]), put, fail);
  await vi.waitFor(() => expect(fail).toHaveBeenCalledWith("chat.drop.failed"));
  expect(put).not.toHaveBeenCalled();
});

it("asks the backend when the webview hides the clipboard, as WebKitGTK does for images", async () => {
  invokeMock.mockResolvedValue(["/private/attachments/pasted.png"]);
  const put = vi.fn();
  const hidden = event([], []);

  pasteFiles(hidden, put, vi.fn());
  expect(hidden.preventDefault).toHaveBeenCalled();
  await vi.waitFor(() => expect(put).toHaveBeenCalledWith(["/private/attachments/pasted.png"]));
});
