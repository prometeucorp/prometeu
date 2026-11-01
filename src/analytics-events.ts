import type { ProviderId } from "./types";

/** Product actions, distinct from control activations. Keep the Rust vocabulary and tracking plan aligned. */
export type FeatureEvent =
  | { name: "settings_section_viewed"; section: "general" | "agents" | "resources" | "actions" | "work" | "analytics" }
  | { name: "settings_search_used" }
  | { name: "preference_changed"; setting: "language" | "dictation_language" | "default_model" | "default_effort" | "notifications" }
  | { name: "resource_action_used"; kind: "mcp" | "plugin" | "skill"; action: "save" | "remove" | "install" | "update" | "copy" | "share" | "authenticate" | "disconnect" | "check" | "discover" | "generate" }
  | { name: "resource_selection_changed"; kind: "mcp" | "plugin" | "skill"; scope: "defaults" | "global" | "workspace" | "project" }
  | { name: "integration_action_used"; integration: "linear" | "github" | "typesafe" | "cloud"; action: "connect" | "disconnect" | "configure" | "test" }
  | { name: "provider_account_action_used"; provider: ProviderId; action: "login_start" | "login_cancel" | "select" | "remove" }
  | { name: "action_configuration_changed"; kind: "command" | "profile" | "project" | "pr_action"; action: "save" | "remove" }
  | { name: "issue_action_used"; integration: "linear" | "github"; action: "claim" | "open_source" | "open_workspace" | "prepare_workspace" }
  | { name: "notification_action_used"; source: "github" | "team" | "local" | "island"; action: "open" | "read" | "read_all" | "open_source" | "open_workspace" | "dismiss" | "answer" }
  | { name: "collaboration_action_used"; action: "select_organization" | "share" | "unshare" | "grant" | "revoke" | "companion_enable" | "companion_disable" | "leave" | "readmit" | "allow_input" | "dismiss_input" | "send_pending" | "edit_pending" | "discard_pending" | "comment" | "reply" | "resolve" }
  | { name: "usage_history_action_used"; action: "view" | "refresh" | "filter" | "export" | "clear" }
  | { name: "update_action_used"; action: "check" | "install" | "restart" }
  | { name: "release_notes_viewed" }
  | { name: "feedback_action_used"; channel: "private" | "public"; action: "open" | "submit"; has_attachment: boolean }
  | { name: "trust_decision_made"; approved: boolean }
  | { name: "desk_layout_changed"; change: "visibility" | "order" | "size" }
  | { name: "conversation_opened"; surface: "desk" | "workspace" }
  | { name: "conversation_resumed" }
  | { name: "attachment_added" }
  | { name: "context_attached" }
  | { name: "output_copied"; kind: "message" | "code" }
  | { name: "request_answered" }
  | { name: "approval_submitted"; approved: boolean }
  | { name: "model_selection_changed"; provider: ProviderId }
  | { name: "model_favorite_changed"; is_favorite: boolean }
  | { name: "effort_changed"; provider: ProviderId }
  | { name: "usage_summary_viewed" }
  | { name: "file_opened"; kind: "text" | "markdown" | "image" | "csv" | "pdf" | "binary" }
  | { name: "file_search_used" }
  | { name: "diff_viewed" }
  | { name: "git_comparison_opened" }
  | { name: "review_note_created" }
  | { name: "review_notes_sent" }
  | { name: "preview_opened" }
  | { name: "browser_context_attached" }
  | { name: "preview_capture_created" }
  | { name: "dictation_started" }
  | { name: "dictation_text_inserted" }
  | { name: "action_used"; kind: "prompt" | "agent" }
  | { name: "machine_panel_opened" }
  | { name: "quota_viewed" }
  | { name: "project_action_used"; action: "add" | "create" | "remove" | "reorder" | "group" | "configure" }
  | { name: "workspace_action_used"; action: "rename" | "pin" | "unpin" | "stage" | "finish" | "archive" | "restore" | "remove" | "cleanup" | "mark_read" | "mark_unread" }
  | { name: "file_operation_completed"; operation: "save" | "create_file" | "create_folder" | "rename" | "move" | "trash" | "restore" }
  | { name: "git_operation_completed"; operation: "stage" | "unstage" | "commit" | "fetch" | "pull" | "push" | "publish" | "discard" | "resolve" }
  | { name: "terminal_opened" }
  | { name: "project_script_started"; kind: "setup" | "run" }
  | { name: "project_script_stopped"; kind: "setup" | "run" }
  | { name: "delegation_action_used"; action: "create" | "send" | "interrupt" | "inspect" | "files" | "setup" | "run" | "preview"; client: "internal" | "external" };
