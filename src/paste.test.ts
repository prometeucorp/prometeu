import { expect, it, vi } from "vitest";
import { invoke } from "./ipc";
import { pasteFiles } from "./paste";

vi.mock("./ipc", () => ({ invoke: vi.fn() }));

const event = (files: unknown[], types = files.length ? ["Files"] : ["text/plain"]) =>
  ({ clipboardData: { files, types }, preventDefault: vi.fn() }) as unknown as ClipboardEvent & { preventDefault: ReturnType<typeof vi.fn> };

it("attaches materialized paths and keeps the default paste for text", async () => {
  vi.mocked(invoke).mockResolvedValue(["/private/attachments/pasted.png"]);
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
  vi.mocked(invoke).mockRejectedValue("chat.drop.failed");
  const put = vi.fn();
  const fail = vi.fn();

  pasteFiles(event([{}]), put, fail);
  await vi.waitFor(() => expect(fail).toHaveBeenCalledWith("chat.drop.failed"));
  expect(put).not.toHaveBeenCalled();
});

it("asks the backend when the webview hides the clipboard, as WebKitGTK does for images", async () => {
  vi.mocked(invoke).mockResolvedValue(["/private/attachments/pasted.png"]);
  const put = vi.fn();
  const hidden = event([], []);

  pasteFiles(hidden, put, vi.fn());
  expect(hidden.preventDefault).toHaveBeenCalled();
  await vi.waitFor(() => expect(put).toHaveBeenCalledWith(["/private/attachments/pasted.png"]));
});
