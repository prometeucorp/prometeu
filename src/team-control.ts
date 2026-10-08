import type { Right } from "./team-rights";

/// Validate peer control before it reaches the agent. Recognize malformed control payloads and discard them so they never become ordinary prompts.

export type RemoteControl = { recognized: boolean; frame: unknown | null };

/// Answering a request needs Control; prompts and interrupts need Send messages (ADR 0090).
export function rightOf(control: RemoteControl): Right {
  const type = (control.frame as { type?: unknown } | null)?.type;
  return type === "request.respond" || type === "control_response" ? "control" : "send";
}

export function remoteControl(data: string): RemoteControl {
  if (!data.startsWith("{")) return { recognized: false, frame: null };
  try {
    const value = JSON.parse(data);
    if (!value || typeof value !== "object" || typeof value.type !== "string") {
      return { recognized: false, frame: null };
    }
    if (value.v === 1) {
      if (value.type === "turn.interrupt") return { recognized: true, frame: value };
      if (value.type === "request.respond") {
        const response = value.response;
        const answers = response?.answers;
        const ok =
          typeof value.requestId === "string" &&
          value.requestId.length > 0 &&
          value.requestId.length <= 128 &&
          (response?.outcome === "allow" ||
            (response?.outcome === "deny" && typeof response.message === "string") ||
            (response?.outcome === "answer" &&
              answers &&
              typeof answers === "object" &&
              Object.values(answers).every((answer) => typeof answer === "string"))) &&
          data.length <= 64 * 1024;
        return { recognized: true, frame: ok ? value : null };
      }
      return { recognized: true, frame: null };
    }
    // Rollback reader for clients on the previous version.
    if (value.type === "control_request") {
      const ok = typeof value.request_id === "string" && value.request_id.length <= 128 && value.request?.subtype === "interrupt";
      return { recognized: true, frame: ok ? value : null };
    }
    if (value.type === "control_response") {
      const response = value.response;
      const answer = response?.response;
      const ok =
        response?.subtype === "success" &&
        typeof response.request_id === "string" &&
        response.request_id.length <= 128 &&
        (answer?.behavior === "allow" || answer?.behavior === "deny") &&
        data.length <= 64 * 1024;
      return { recognized: true, frame: ok ? value : null };
    }
    return { recognized: value.type.startsWith("control_"), frame: null };
  } catch {
    return { recognized: false, frame: null };
  }
}
