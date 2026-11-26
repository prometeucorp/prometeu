---
type: Contract
title: "Browser and visual context"
description: "The preview, the inspection, the captures and how they go into the draft."
status: stable
---

# Browser and visual context

The app obeys this contract. [ADR 0037](../decisions/0037-browser-design-context.md)
records the decision.

## Presentation and lifecycle

Each local workspace keeps a native `run-<id>` webview. The conversation stays
in view adjacent to the preview. When a person changes to a different
conversation, the page stays. Files, Changes and terminals replace that
composition. When a person closes the browser, the app removes the webview.
When a person goes out of the workspace, the app only hides the webview.

`src/browser.ts` puts these operations in sequence: open, position, hide and
close. A generation number cancels previous open operations and URL queries. A
selection generation number also cancels captures after a new inspection, a
navigation or a change of width. Menus, dialogs, popovers and feedback stop the
native view for a short time. A resize of the divided view also stops it, so
that the view continues to get the pointer events. When the browser opens, the
app closes the right panel of Files, Changes, Review and the terminals. This gives space to the
conversation and the page. During the preview, a person can open the panel
again with the panel button. When a person goes out of the browser or the
workspace, the panel gets its previous visibility again.

The preview has resize observers and body-mutation observers for its position,
and a toggle listener. The app connects them only when a preview is open. They
stay connected when a menu or a dialog hides the native view for a short time.
Thus, the view can get its position and dimensions again when the menu or the
dialog closes. When
the app hides the preview or a person goes out of it, the app disconnects them
and cancels a queued position frame. When a different preview opens, the app
connects them again after the native view opens. A previous open operation in the
sequence cannot change the position of the new workspace.

An explicit local MCP request can go to a delegated conversation that the
client owns. It can open this same preview through the `workspace-preview`
event. The desktop examines the availability again and makes the URL from the
port of the workspace. The event cannot supply a URL or a script. The open
operation does not start a service and does not tell that the service operates.
See [embedded MCP](embedded-mcp.md).

The selected width is a maximum. The available space limits it. There is no
device emulation, no user-agent emulation and no viewport larger than the
native surface.

`browser_url` keeps its `string | null` response. On macOS, it reads the native
URL on the AppKit thread. It returns `null` when the URL or its absolute string
is missing, incorrect, or not available in 3 seconds. This prevents the
nullable-URL unwrap of Wry 0.55.1 when the app reads the URL at intervals. When
there is no result, the address bar does not change, and the app cannot make
sure of the URL of an inspection. On other platforms, the app keeps the Tauri URL query.

## Additive IPC

The previous commands for the navigation and the view dimensions stay
compatible. The new commands are in
the Rust registry, in `src/ipc.ts` and in the mock:

| Command | Arguments | Return |
| --- | --- | --- |
| `browser_inspect` | `{ id, enabled }` | `void` |
| `browser_selection` | `{ id }` | `{ active, selection }` |
| `browser_capture` | `{ id, rect? }` | absolute path of a private PNG |

`selection` is `null` or:

```ts
{
  url: string;
  selector: string;
  tag: string;
  text: string;
  html: string;
  styles: Record<string, string>;
  rect: { x: number; y: number; width: number; height: number };
  viewport: { width: number; height: number };
}
```

`rect` uses CSS pixels relative to the viewport of the page. When a selector
goes through an open Shadow DOM, ` >>> ` divides the hosts. That convention is
text context, not one CSS selector. The app does not examine the internal
content of iframes and of a closed Shadow DOM. The app does not promise a
relation with components or source files.

## Trust and limits

The backend injects only the script from `src/browser-inspector.js`, and this
script does not change. No
command receives arbitrary JavaScript, and the app adds no remote capability.
The app does not trust the page. Rust deserializes and limits the result before
it gives it to the interface. The interface shows HTML and CSS with
`textContent`.

On macOS, the Run child webview rejects all permission requests for media
capture. It stays on `about:blank` until the app installs that native delegate
override. If the app cannot install the limit, the webview closes. The primary
webview keeps the media behavior of Wry, because dictation in the composer must
have access to the microphone. Remote HTTP and HTTPS pages do not share that
access.

The script sets these limits:

- HTML: 12,000 UTF-16 units.
- Text: 2,000 units.
- The selector: 1,000 units.
- The URL: 4,096 units.
- Each style: 1,000 units.

The script removes form values, handlers and executable content from the copied
part of the page. Rust accepts the related UTF-8 dimension and limits the object to
256 KiB. Rust refuses unknown fields, URLs that are not HTTP(S), incorrect
geometry and more than 64 style properties.

The script has a 3-second timeout. The macOS capture uses the
public `WKWebView.takeSnapshot` API. It does not capture all of the screen, and
the screen recording permission is not necessary. It cuts the selection to the
current view. Without `rect`, it captures the viewport that a person can see. It has a
5-second timeout and a 20 MiB PNG limit. It writes to
`<root>/attachments/<uuid>/browser.png` with the current private permissions.

Before and after the cut, the capture makes sure that the element continues to
have a connection to the page. The element must have the same geometry, URL and viewport, without
a scroll or a resize after the selection. After a change, the app discards only
the PNG and keeps the selected text context. This does not stop animations, and
it does not promise that the visual content of the page stays the same. On other
platforms than macOS, the capture returns `err.browser.captureFailed`.
Inspection failures return `err.browser.inspectFailed`.

When a person selects an element, the app prepares the context and the capture.
**Add to chat** makes a **Selected element** tag that is different from the
typed text. The tag holds the data and the PNG. The person can examine the
data and can remove all of the tag before the send. Drafts keep these tags
for each conversation, also between the desk and the workspace. After a capture
failure, the tag stays with the text data. A capture without a selection
records the target conversation before the IPC call. It uses the current lock
for attachments that wait.

## Context in the message text

[Decision 0038](../decisions/0038-browser-context-chips.md) keeps IPC,
Conversation Events V1 and the relay without changes. Only at the send does each
tag become an identified block in `text`:

```text
<prometeu-browser-element v="1">
{"selection":{...},"image":"/path/browser.png"}
@"/path/browser.png"
</prometeu-browser-element>
```

The full JSON is on one line. A `<` in the values becomes `\u003c`. `selection`
follows the DTO above. Without a capture, the block does not include `image`,
and the mention line is empty. The mention line uses the attachment mechanism that
the agents know. When a person opens a history or a shared conversation, the
app does not read local files automatically. The blocks come before the typed
text, as usual attachments do. Thus, when a text starts with `/` and becomes a
provider command, the context stays.

`src/browser-context.ts` accepts only a correct version, structure and limits,
with a mention line that agrees with the declared PNG. Blocks that are incorrect
or unknown stay as literal text. The desktop and the phone show the correct
blocks as tags in the history. They also show them while a send waits. The
reducer and the adapters keep the full data. Previous clients show the raw text. The app does not
write previous transcripts again and does not migrate data.

A person can send only tags, can send them with text and other attachments, or
can use them as context in Actions. The send keeps the current error behavior
and the stored queue. The presentation does not put messages in the queue
again.

## Drag

Only the primary view changes local files and promises into the current
`file-drag` event. The child view disables the drag interception of Tauri. Thus,
the page can receive uploads. When a person releases an item on the chat, the
item follows the attachment flow. When a person releases an item on the preview,
the page receives it. This prevents a problem: Tauri can get the upload and then
ignore its event, because the event did not come from `main`.

## Evidence and verification limits

- `e2e/browser-inspector.spec.ts`: the selection of the installed script, Escape,
  navigation, sanitization, Shadow DOM, scroll and resize in Chromium. It also
  does some engine checks in WebKit.
- `e2e/browser.spec.ts`: composition, drafts, attachments, navigation and
  lifecycle through the web mock. It also examines tags and the actual content
  that the app sends to the agent.
- `src/browser-context.test.ts`: text compatibility. It also makes sure that
  the app rejects incorrect blocks and continues to show usual content.
- `e2e/mobile.spec.ts`: tags and their data in the shared history on a narrow
  screen.
- `src-tauri/src/browser.rs`: DTO validation, cuts, limits and the macOS
  nullable-URL conversion. It keeps empty pages and HTTP(S) navigation. The URL
  unit test does not copy the time sequence of the native navigation or a
  WebKit process failure.
- `e2e/file-drop.spec.ts`: the attachment and promise contract in the frontend.

`src/mock-browser.ts` uses an iframe only in web development. It has a
controlled page and the same inspection script. Captures return paths to files that are not
there. These tests do not prove the native PNG, the AppKit gesture or how the
CLI uses the image. In this environment, the Accessibility permission stops the
verification of the native gesture. This is not evidence that the gesture operates
correctly.

Review notes use a second text envelope with a version, through the shared
message splitter. See
[Local diff review](diff-review.md#text-envelope-and-compatibility).
