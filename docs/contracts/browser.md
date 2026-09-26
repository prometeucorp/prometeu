# Browser and visual context

Status: implemented. Decision: [ADR 0037](../decisions/0037-browser-design-context.md).

## Presentation and lifecycle

Each local workspace keeps a native `run-<id>` webview. The conversation stays
visible next to the preview; switching conversations preserves the page. Files,
Changes and terminals replace that composition. Closing the browser destroys the
webview; leaving the workspace only hides it.

`src/browser.ts` serializes opening, positioning, hiding and closing. A
generation invalidates old openings and URL queries. A selection generation also
invalidates captures after a new inspection, navigation or width change. Menus,
dialogs, popovers and feedback suspend the native view; resizing the split also
suspends it, to preserve pointer events. Opening the browser collapses the right
panel of Files, Changes, Review and supporting terminals to give room to the
conversation and the page. The panel button allows reopening it during the
preview. Leaving the browser or the workspace restores the panel's previous
visibility.

The preview's resize and body-mutation positioning observers, plus the toggle
listener, are connected only while a preview is open. They remain connected
while a menu or dialog temporarily conceals the native view so its bounds can
recover on close. Hiding or leaving disconnects them and cancels a queued
position frame. Opening another preview reconnects them after the native view
opens; an old serialized open cannot reposition the new workspace.

An explicit local MCP request may navigate to an owned delegated conversation
and open this same preview through the `workspace-preview` event. The desktop
rechecks availability and derives the URL from the workspace port; the event
cannot supply a URL or script. Opening neither starts a service nor certifies
readiness. See [embedded MCP](embedded-mcp.md).

The chosen width is a maximum, limited by the available space. There is no
device emulation, user-agent emulation or viewport larger than the native
surface.

## Additive IPC

The old navigation and bounds commands remain compatible. New commands exist in
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

`rect` uses CSS pixels relative to the page viewport. Selectors crossing an open
Shadow DOM separate hosts with ` >>> `; that convention is textual context, not
a single CSS selector. The internal content of iframes and of a closed Shadow
DOM is not inspected. There is no guaranteed association with components or
source files.

## Trust and limits

The backend injects only the fixed script from `src/browser-inspector.js`. No
command receives arbitrary JavaScript and no remote capability is added. The
page stays untrusted: Rust deserializes and limits the result before delivering
it to the interface. HTML and CSS are displayed with `textContent`.

On macOS, the Run child webview denies every media-capture permission request.
It remains on `about:blank` until that native delegate override is installed and
closes if the restriction cannot be installed. The main webview keeps Wry's
media behavior because composer dictation needs microphone access; remote HTTP
and HTTPS pages never share that access.

The script limits HTML to 12,000 UTF-16 units, text to 2,000, the selector to
1,000 and the URL to 4,096; each style has up to 1,000. It removes form values,
handlers and executable content from the copied excerpt. Rust accepts the
corresponding UTF-8 size, limits the object to 256 KiB and refuses unknown
fields, URLs outside HTTP(S), invalid geometry and more than 64 style
properties.

Evaluation has a 3-second timeout. The macOS capture uses the public
`WKWebView.takeSnapshot` API, without capturing the whole screen and without
screen recording permission. It crops the selection to the current bounds;
without `rect`, it captures the visible viewport. It has a 5-second timeout, a
20 MiB PNG limit and writes to `<root>/attachments/<uuid>/browser.png` with the
existing private permissions. Before and after cropping, it confirms that the
element is still connected, with the same geometry, URL and viewport, without
scroll or resize since the selection. A change discards only the PNG, preserving
the chosen textual context. This does not freeze animations and does not
guarantee that the page's visual content stays still. Outside macOS, capture
returns `err.browser.captureFailed`. Inspection failures return
`err.browser.inspectFailed`.

Selecting an element prepares context and capture. **Add to chat** creates a
**Selected element** tag, separate from the typed text. The tag gathers data and
PNG, allows reviewing details and can be removed as a whole before sending.
Drafts keep these tags per conversation, including between the desk and the
workspace. A capture failure preserves the tag with the textual data. A
standalone capture retains the conversation target before the IPC call and uses
the existing pending-attachment lock.

## Context in the message text

[Decision 0038](../decisions/0038-browser-context-chips.md) keeps IPC,
Conversation Events V1 and the relay unchanged. Only on send does each tag
become an identified block inside `text`:

```text
<prometeu-browser-element v="1">
{"selection":{...},"image":"/path/browser.png"}
@"/path/browser.png"
</prometeu-browser-element>
```

The complete JSON takes one line; `<` in the values becomes `\u003c`.
`selection` follows the DTO above. Without a capture, `image` is omitted and the
mention line is empty. The mention keeps the attachment mechanism the agents
already understand. There is no automatic local reading when opening a history
or a shared conversation. The blocks precede the typed text, just like ordinary
attachments, so that a text starting with `/` does not discard the context when
it becomes a provider command.

`src/browser-context.ts` recognizes only a valid version, structure and limits,
with a mention matching the declared PNG. Invalid or unknown blocks stay as
literal text. Desktop and phone present the valid blocks as tags in the history
too, and while a send is pending. The reducer and the adapters keep the complete
data; old clients show the raw text. There is no rewriting of previous
transcripts and no data migration.

It is possible to send only tags, to combine them with text and other
attachments, or to use them as context in Actions. Sending keeps the existing
handling of errors and of the persisted queue; the presentation does not requeue
messages.

## Drag

Only the main view adapts local files and promises to the existing `file-drag`
event. The child view disables Tauri's drag interception to allow uploads inside
the page. Dropping on the chat follows the attachment flow; dropping on the
preview belongs to the page. This prevents Tauri from consuming the upload and
then ignoring its event because it did not come from `main`.

## Evidence and verification limits

- `e2e/browser-inspector.spec.ts`: the real script's selection, Escape,
  navigation, sanitization, Shadow DOM, scroll and resize in Chromium, with
  selected engine checks in WebKit.
- `e2e/browser.spec.ts`: composition, drafts, attachments, navigation and
  lifecycle over the web mock, tags and the actual content sent to the agent.
- `src/browser-context.test.ts`: textual compatibility and rejection of invalid
  blocks without hiding ordinary content.
- `e2e/mobile.spec.ts`: tags and details in the shared history on a narrow
  screen.
- `src-tauri/src/browser.rs`: DTO validation, crops and limits.
- `e2e/file-drop.spec.ts`: attachment and promise contract in the frontend.

`src/mock-browser.ts` uses an iframe only in web development, with a controlled
page and the same inspection script. Captures return fictional paths. These
tests do not prove the native PNG, the AppKit gesture or the consumption of the
image by the CLI. Verifying the native gesture in this environment remains
blocked by the Accessibility permission; that is not evidence that the gesture
was approved.
