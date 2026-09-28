# Cross-family, read-only review Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `/review` starts on a provider family other than the one that built the workspace when another is installed and signed in, runs read-only by construction in Claude and Codex, and the task tab says which provider reviewed, whether read-only is enforced and whether it fell back to the same family.

**Architecture:** Profiles gain `provider_rule`, ordered `candidates` and `access`. `action_start` picks the provider with a pure function before resolving tools, freezes the picked candidate as a fixed choice and records `same_family`. `Launch.access` reaches the adapters: Claude uses restricted `dontAsk` mode with read tools, Codex uses the `read-only` sandbox, Antigravity refuses. Presentation reads the new `readOnlyProfile` capability.

**Tech Stack:** Rust (Tauri backend, serde, cargo test), TypeScript (Vite, Vitest), Playwright (existing E2E only).

**Spec:** [`docs/superpowers/specs/2026-09-28-cross-family-review-design.md`](../specs/2026-09-28-cross-family-review-design.md)

## Global Constraints

- Code identifiers, comments and tests in English; visible text through i18n in both `src/i18n.en.ts` and `src/i18n.pt.ts`; user data and agent output stay untouched.
- Presentation decides through `AgentCapabilities`; never compare provider-id literals in presentation (`npm run architecture:check`).
- Persisted fields are additive with `#[serde(default)]`; boards, catalogs and frozen `Tab.task` written before this change keep loading.
- `action_start({ workspace, name, context }) -> Tab` keeps its signature; `src/mock.ts` mirrors every behavior the browser can observe.
- Claude read-only flags, exactly: `--restricted --permission-mode dontAsk --tools Read,Grep,Glob,Bash,Skill,Task --add-dir <worktree>`, environment `CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD=1`, and `--strict-mcp-config --mcp-config {"mcpServers":{}}`; never `--dangerously-skip-permissions`, `--allow-dangerously-skip-permissions` or plan mode.
- Codex read-only params, exactly: `"sandbox": "read-only"`, `"approvalPolicy": "never"` on `thread/start` and `thread/resume`, plus `-c mcp_servers={}`.
- `different_from_builder` requires 1–3 candidates with distinct, runnable providers and `choice == candidates[0]`; `fixed` requires no candidates.
- Read-only requires `readOnlyProfile` for every provider the profile can start with and an MCP selection that is `null` or empty; resolution freezes MCP as empty.
- Bundled seed revision 2: `different_from_builder`, candidates `[codex, claude]` with empty model and effort, `access: read_only`, the existing prompt unchanged.
- Never commit the unrelated local change in `package-lock.json`; stage files explicitly.
- Markdown snippets in this plan write a link as `⟦text | path⟧`, because their paths resolve from the target document, not from this plan; write each one as an ordinary inline Markdown link with that text and path in the target file, so `npm run docs:check` validates it there.
- Commits follow Conventional Commits in English with a lowercase description, no trailing period, and end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

- A catalog written by an older app or pulled from the Cloud without the new fields must load as `fixed`/`default`, and the seed upgrade must never loop or overwrite a customized profile — Task 2 adds `older_catalogs_load_and_upgrade_only_the_untouched_seed`.
- A workspace whose only tabs are earlier task tabs (a finished `/review`) must not count the reviewer as a builder — Task 3's `builders` test covers task-only and empty workspaces.
- A shell that exports `CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD=0` must still load `CLAUDE.md`, because `account_env` strips inherited `CLAUDE*` variables before the read-only one is set — Task 4 adds `read_only_memory_survives_inherited_claude_variables`.
- Ordinary sessions must keep today's flags and params (bypass under `auto`, `danger-full-access`) — Tasks 4 and 5 assert the default branches.
- A read-only profile that points at Antigravity or keeps an MCP selection must be refused with an explanation, not saved or silently trimmed — Task 2 tests `validRules`, Task 6 shows the reason.

## File Structure

| File | Responsibility |
| --- | --- |
| `src-tauri/src/agents.rs` | `AgentCapabilities.read_only_profile`; `usable(ProviderId)` availability glue |
| `src-tauri/src/actions.rs` | `ProviderRule`, `Access`, new `Profile`/`Run`/`Catalog` fields, validation, seed upgrade, `resolve`, `action_start` |
| `src-tauri/src/actions/reviewer.rs` (new) | pure `builders` and `pick` |
| `src-tauri/src/accounts.rs` | `signed_in(ProviderId)` over the registry |
| `src-tauri/src/platform.rs` | `has(program)` available on macOS too |
| `src-tauri/src/session.rs` | `Launch.access`; task launches carry the frozen access |
| `src-tauri/src/claude.rs`, `claude/fixtures/*` (new) | read-only arguments, environment, recording |
| `src-tauri/src/codex.rs`, `codex/contract.rs`, `codex/fixtures/*` | `Start.access`, sandbox params, recording |
| `src-tauri/src/antigravity.rs` | refuse read-only launches |
| `src/action-defaults.json` | seed revision 2 plus the previous seed |
| `src/actions.ts` | TS types, seed parity, `builders`, `pick`, `validRules`, `taskBadges` |
| `src/agents.ts`, `src/mock.ts` | capability type/defaults; mock parity |
| `src/model-picker.ts` | `accepts` filter |
| `src/action-settings.ts`, `src/style.css` | profile editor and cards |
| `src/components/chat/composer.ts`, `chat.css`, `src/chat.ts`, `src/components/stories.ts`, `src/components/catalog.json` | task footer label and badges |
| `docs/…`, `README.md` | ADR 0064, contracts, matrix, indexes |

---

### Task 1: Read-only capability in the provider descriptor

**Files:**
- Modify: `src-tauri/src/agents.rs:46-59, 88-119, 254-294`
- Modify: `src/agents.ts:11-21, 47-57`
- Modify: `src/agents.test.ts:6`
- Modify: `src/mock.ts:1674-1729`
- Regenerate: `fixtures/backend-contract.json`
- Modify: `docs/contracts/agent-runtime.md:105-133`

**Interfaces:**
- Produces: `agents::AgentCapabilities { read_only_profile: bool, .. }` (JSON `readOnlyProfile`); `crate::agents::capabilities(id).read_only_profile`; TS `AgentCapabilities.readOnlyProfile: boolean`.

- [ ] **Step 1: Write the failing Rust test**

In `src-tauri/src/agents.rs`, extend `capabilities_belong_to_the_descriptor_not_the_view` and `antigravity_advertises_only_supported_controls_and_external_account`:

```rust
        assert!(claude.capabilities.read_only_profile);
        assert!(codex.capabilities.read_only_profile);
```

(add after `assert!(codex.capabilities.resume);`) and, in the JSON part of the same test:

```rust
        assert_eq!(json["capabilities"]["readOnlyProfile"], true);
```

and in the Antigravity test after `assert!(!g.capabilities.workspace_plugin_selection);`:

```rust
        assert!(!g.capabilities.read_only_profile);
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd src-tauri && cargo test --lib agents::tests`
Expected: compile error `no field read_only_profile on type AgentCapabilities`.

- [ ] **Step 3: Implement the capability**

In `AgentCapabilities` add the field after `attachments`:

```rust
    /// The adapter enforces a read-only task profile by construction instead of by instruction.
    pub read_only_profile: bool,
```

In `capabilities()`, add to `common` (Claude and Codex inherit it):

```rust
        // Claude's restricted dontAsk mode and Codex's read-only sandbox remove write access;
        // see actions::Access.
        read_only_profile: true,
```

and in the Antigravity/RetiredGemini arm add `read_only_profile: false,`.

- [ ] **Step 4: Update the TypeScript side**

`src/agents.ts`: add `readOnlyProfile: boolean;` to `AgentCapabilities` after `attachments`, and `readOnlyProfile: false,` to `NO_CAPABILITIES`.

`src/agents.test.ts` line 6: add `readOnlyProfile: true` to `common`.

`src/mock.ts` `agents()`: add `readOnlyProfile: true,` to the Claude and Codex capability objects and `readOnlyProfile: false` to Antigravity's.

- [ ] **Step 5: Regenerate the contract fixture and run the checks**

Run: `npm run contracts:update && cd src-tauri && cargo test --lib agents::tests && cd .. && npm run test:contracts && npx vitest run src/agents.test.ts && npm run typecheck`
Expected: all PASS; `git diff fixtures/backend-contract.json` shows only `"readOnlyProfile"` lines (true for claude/codex, false for antigravity/gemini).

- [ ] **Step 6: Document the capability**

In `docs/contracts/agent-runtime.md`, add `readOnlyProfile: boolean;` after `attachments: boolean;` in the capabilities block, and after the paragraph ending "Protocol details, such as the name of a JSON-RPC method, are not capabilities." add:

```md
`readOnlyProfile` means the adapter can run a task profile whose `access` is
`read_only` with write access removed by the CLI itself; see
⟦actions | actions.md#access-level⟧. Claude and Codex advertise it; Antigravity
does not.
```

Run: `npm run docs:check` → PASS.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/agents.rs src/agents.ts src/agents.test.ts src/mock.ts fixtures/backend-contract.json docs/contracts/agent-runtime.md
git commit -m "refactor(agents): add the read-only profile capability" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

(The `docs/contracts/actions.md#access-level` anchor is created in Task 8; `docs:check` ignores anchors.)

---

### Task 2: Provider rule, access level and the one-time seed upgrade

**Files:**
- Modify: `src-tauri/src/state.rs:84` (derive `Debug` on `Choice`)
- Modify: `src-tauri/src/actions.rs:10-65, 116-133, 581-640`
- Modify: `src/action-defaults.json`
- Modify: `src/actions.ts:1-28`
- Modify: `src/actions.test.ts`
- Modify: `src/action-settings.ts:135-138, 242`
- Modify: `src/mock.ts:1063-1072`
- Modify: `src-tauri/src/session.rs:3554-3567`
- Modify: `e2e/actions.spec.ts:47-54`

**Interfaces:**
- Consumes: `crate::agents::capabilities(id).read_only_profile` (Task 1).
- Produces (Rust, `crate::actions`): `enum ProviderRule { Fixed, DifferentFromBuilder }` (`"fixed" | "different_from_builder"`), `enum Access { Default, ReadOnly }` (`"default" | "read_only"`), both `Copy + Default + Debug + PartialEq`; `Profile { provider_rule: ProviderRule, candidates: Vec<Choice>, access: Access, .. }`; `Catalog { defaults_revision: u32, .. }`; `validate_profile` enforces the rule/access invariants.
- Produces (TS, `src/actions.ts`): `type ProviderRule`, `type Access`, `Profile.provider_rule/candidates/access` (required), `Catalog.defaults_revision?: number`, `validRules(profile: Profile, readOnlyCapable: (agent: ProviderId) => boolean): boolean`, `initializeDefaults` with the upgrade.

- [ ] **Step 1: Write the failing Rust tests**

In `src-tauri/src/actions.rs` tests, change `profile()` to add the new fields:

```rust
            provider_rule: ProviderRule::Fixed,
            candidates: vec![],
            access: Access::Default,
```

Replace the customization step of `code_review_defaults_migrate_once_and_preserve_user_choices` (it used to change the model, which would now break the candidate invariant):

```rust
        board.actions.profiles[0].prompt = "Custom review".into();
        board.revive();
        assert_eq!(board.actions.profiles.len(), 1);
        assert_eq!(board.actions.profiles[0].prompt, "Custom review");
```

and add these tests:

```rust
    fn seed() -> serde_json::Value {
        serde_json::from_str(include_str!("../../src/action-defaults.json")).unwrap()
    }

    #[test]
    fn the_bundled_reviewer_is_cross_family_and_read_only() {
        let mut c = Catalog::default();
        c.initialize_defaults();
        let p = &c.profiles[0];
        assert_eq!(p.provider_rule, ProviderRule::DifferentFromBuilder);
        assert_eq!(p.access, Access::ReadOnly);
        let agents: Vec<_> = p.candidates.iter().map(|c| c.agent).collect();
        assert_eq!(agents, [ProviderId::Codex, ProviderId::Claude]);
        assert!(p.candidates[0] == p.choice);
        assert_eq!(c.defaults_revision, seed()["revision"].as_u64().unwrap() as u32);
        assert!(validate(&c).is_ok());
    }

    #[test]
    fn older_catalogs_load_and_upgrade_only_the_untouched_seed() {
        let command = seed()["command"].clone();
        let previous = seed()["previous"][0].clone();
        let catalog = |profile: serde_json::Value| -> Catalog {
            // A catalog written before this change: no rule, candidates, access or revision.
            serde_json::from_value(json!({
                "defaults_initialized": true, "profiles": [profile], "commands": [command]
            }))
            .unwrap()
        };
        let mut untouched = catalog(previous.clone());
        assert_eq!(untouched.profiles[0].provider_rule, ProviderRule::Fixed);
        assert_eq!(untouched.profiles[0].access, Access::Default);
        untouched.initialize_defaults();
        assert_eq!(untouched.profiles[0].access, Access::ReadOnly);
        assert_eq!(untouched.profiles[0].choice.agent, ProviderId::Codex);
        // The upgrade runs once; returning to the old shape later is the person's choice.
        let mut reverted: Catalog =
            serde_json::from_str(&serde_json::to_string(&untouched).unwrap()).unwrap();
        reverted.profiles[0] = serde_json::from_value(previous.clone()).unwrap();
        reverted.initialize_defaults();
        assert_eq!(reverted.profiles[0].choice.agent, ProviderId::Claude);
        assert_eq!(reverted.profiles[0].access, Access::Default);
        let mut custom = previous;
        custom["choice"]["model"] = json!("opus");
        let mut customized = catalog(custom);
        customized.initialize_defaults();
        assert_eq!(customized.profiles[0].choice.model, "opus");
        assert_eq!(customized.profiles[0].provider_rule, ProviderRule::Fixed);
        assert_eq!(customized.defaults_revision, untouched.defaults_revision);
    }

    #[test]
    fn provider_rules_and_read_only_access_are_validated() {
        let valid = |p: &Profile| validate_profile(p).is_ok();
        let choice = |agent| Choice { agent, ..Default::default() };
        let mut p = profile();
        p.watch = None;
        assert!(valid(&p));
        p.candidates = vec![choice(ProviderId::Claude)];
        assert!(!valid(&p), "a fixed profile carries no candidates");
        p.provider_rule = ProviderRule::DifferentFromBuilder;
        p.choice = choice(ProviderId::Codex);
        p.candidates = vec![choice(ProviderId::Codex), choice(ProviderId::Claude)];
        assert!(valid(&p));
        p.choice = choice(ProviderId::Claude);
        assert!(!valid(&p), "choice mirrors the first candidate");
        p.choice = choice(ProviderId::Codex);
        p.candidates.push(choice(ProviderId::Codex));
        assert!(!valid(&p), "providers are distinct");
        p.candidates = vec![choice(ProviderId::Codex), choice(ProviderId::RetiredGemini)];
        assert!(!valid(&p), "retired providers cannot run");
        p.candidates = vec![];
        assert!(!valid(&p), "the rule needs a candidate");
        p.candidates = vec![choice(ProviderId::Codex), choice(ProviderId::Antigravity)];
        assert!(valid(&p));
        p.access = Access::ReadOnly;
        assert!(!valid(&p), "Antigravity cannot enforce read-only access");
        p.candidates.pop();
        assert!(valid(&p));
        p.mcp = Some(vec!["server".into()]);
        assert!(!valid(&p), "MCP tools run outside the read-only envelope");
        p.mcp = Some(vec![]);
        assert!(valid(&p));
    }
```

In `src-tauri/src/session.rs` test `task_launch_uses_project_override_and_freezes_tools_and_model`, add to the `Profile` literal: `provider_rule: crate::actions::ProviderRule::Fixed, candidates: vec![], access: crate::actions::Access::Default,`.

- [ ] **Step 2: Run them to verify they fail**

Run: `cd src-tauri && cargo test --lib actions::tests`
Expected: compile errors for `ProviderRule`, `Access` and the new fields.

- [ ] **Step 3: Implement the Rust model**

`src-tauri/src/state.rs`: `#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]` on `Choice`.

`src-tauri/src/actions.rs`: change the import to `use crate::state::{publish, Choice, ProviderId, Status, Tab};`. Add to `Catalog` after `defaults_initialized`:

```rust
    /// Seed revision already applied. Absent in catalogs from before revisions, which the upgrade
    /// treats as revision 1, ADR 0010's profile.
    #[serde(default)]
    pub defaults_revision: u32,
```

Replace `initialize_defaults`:

```rust
    /// Seed once so removing or customizing the profile survives later startups. A catalog seeded
    /// by an earlier revision gets the current profile only while it is still identical to an
    /// earlier seed; the revision records that the upgrade ran, so it never runs twice.
    pub fn initialize_defaults(&mut self) {
        #[derive(Deserialize)]
        struct Seed {
            revision: u32,
            profile: Profile,
            command: Action,
            previous: Vec<Profile>,
        }
        let seed: Seed = serde_json::from_str(include_str!("../../src/action-defaults.json"))
            .expect("valid bundled action defaults");
        if self.defaults_initialized && self.defaults_revision >= seed.revision {
            return;
        }
        if !self.defaults_initialized {
            if !self.profiles.iter().any(|p| p.id == seed.profile.id)
                && !self.commands.iter().any(|c| c.name == seed.command.name)
            {
                self.profiles.push(seed.profile);
                self.commands.push(seed.command);
            }
        } else if let Some(profile) = self
            .profiles
            .iter_mut()
            .find(|p| seed.previous.contains(p))
        {
            *profile = seed.profile;
        }
        self.defaults_initialized = true;
        self.defaults_revision = seed.revision;
    }
```

After `Permission`, add:

```rust
/// How a task picks its provider when it starts.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ProviderRule {
    /// Always `choice`.
    #[default]
    Fixed,
    /// The first usable candidate whose provider did not build the workspace; see reviewer.rs.
    DifferentFromBuilder,
}

/// What a task may change. Adapters materialize `ReadOnly`; `permission` applies only to
/// `Default`.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    #[default]
    Default,
    ReadOnly,
}
```

In `Profile`, after `choice`:

```rust
    #[serde(default)]
    pub provider_rule: ProviderRule,
    /// Ordered candidates for `DifferentFromBuilder`, each with its own model and effort. `choice`
    /// mirrors the first, so an app that ignores these fields runs it as a fixed profile.
    #[serde(default)]
    pub candidates: Vec<Choice>,
    #[serde(default)]
    pub access: Access,
```

Before `validate_profile`, add:

```rust
/// The providers a profile can start with.
fn providers(p: &Profile) -> Vec<ProviderId> {
    match p.provider_rule {
        ProviderRule::Fixed => vec![p.choice.agent],
        ProviderRule::DifferentFromBuilder => p.candidates.iter().map(|c| c.agent).collect(),
    }
}

fn valid_rule(p: &Profile) -> bool {
    match p.provider_rule {
        ProviderRule::Fixed => p.candidates.is_empty(),
        ProviderRule::DifferentFromBuilder => {
            (1..=3).contains(&p.candidates.len())
                && p.candidates[0] == p.choice
                && p.candidates.iter().enumerate().all(|(i, c)| {
                    c.agent != ProviderId::RetiredGemini
                        && !p.candidates[..i].iter().any(|other| other.agent == c.agent)
                })
        }
    }
}

/// Every provider the profile can start with must enforce read-only access, and MCP tools run
/// outside that envelope, so an explicit server selection contradicts it.
fn valid_access(p: &Profile) -> bool {
    p.access == Access::Default
        || (providers(p)
            .into_iter()
            .all(|agent| crate::agents::capabilities(agent).read_only_profile)
            && p.mcp.as_ref().is_none_or(Vec::is_empty))
}
```

and extend the condition in `validate_profile` with `|| !valid_rule(p) || !valid_access(p)` before the `{`.

- [ ] **Step 4: Update the shared seed**

Replace `src/action-defaults.json` with revision 2. The prompt is the current one, byte-identical in both profiles:

```json
{
  "revision": 2,
  "profile": {
    "id": "prometeu-code-review",
    "name": "Code review",
    "prompt": "Review the changes in this workspace against each repository's base branch. Follow repository instructions and use an available code-review skill when applicable. Focus on correctness, regressions, security, and missing validation. Inspect the code and relevant tests before reporting findings. Report only actionable findings, with severity, file and line, a concrete failure scenario, and a suggested fix. If there are no findings, say so and identify any remaining validation gaps. Do not modify files, commit, push, open a PR, or merge. Finish after delivering the review. Respond in the language used by the person requesting the review.",
    "choice": { "agent": "codex", "model": "", "effort": "" },
    "provider_rule": "different_from_builder",
    "candidates": [
      { "agent": "codex", "model": "", "effort": "" },
      { "agent": "claude", "model": "", "effort": "" }
    ],
    "access": "read_only",
    "mcp": null,
    "plugins": null,
    "skills": [],
    "permission": "ask",
    "watch": null
  },
  "command": { "name": "review", "description": "", "kind": "agent", "prompt": "", "profile": "prometeu-code-review" },
  "previous": [
    {
      "id": "prometeu-code-review",
      "name": "Code review",
      "prompt": "Review the changes in this workspace against each repository's base branch. Follow repository instructions and use an available code-review skill when applicable. Focus on correctness, regressions, security, and missing validation. Inspect the code and relevant tests before reporting findings. Report only actionable findings, with severity, file and line, a concrete failure scenario, and a suggested fix. If there are no findings, say so and identify any remaining validation gaps. Do not modify files, commit, push, open a PR, or merge. Finish after delivering the review. Respond in the language used by the person requesting the review.",
      "choice": { "agent": "claude", "model": "", "effort": "" },
      "mcp": null,
      "plugins": null,
      "skills": [],
      "permission": "ask",
      "watch": null
    }
  ]
}
```

- [ ] **Step 5: Run the Rust tests**

Run: `cd src-tauri && cargo test --lib actions:: && cargo test --lib session::tests::task_launch_uses_project_override_and_freezes_tools_and_model`
Expected: PASS.

- [ ] **Step 6: Write the failing TypeScript tests**

Add to `src/actions.test.ts` (import `defaultsText from "./action-defaults.json?raw"` and `validRules, type Catalog, type Profile` from `./actions`):

```ts
describe("provider rule and access", () => {
  const seed = JSON.parse(defaultsText) as { revision: number; profile: Profile; command: Action; previous: Profile[] };
  const old = (): Catalog => ({ ...emptyCatalog(), defaults_initialized: true, profiles: [structuredClone(seed.previous[0])], commands: [seed.command] });
  it("upgrades an untouched earlier seed exactly once and keeps customized profiles", () => {
    const upgraded = initializeDefaults(old());
    expect(upgraded.defaults_revision).toBe(seed.revision);
    expect(upgraded.profiles[0]).toMatchObject({ provider_rule: "different_from_builder", access: "read_only" });
    expect(upgraded.profiles[0].candidates.map(c => c.agent)).toEqual(["codex", "claude"]);
    upgraded.profiles[0] = structuredClone(seed.previous[0]);
    expect(initializeDefaults(upgraded).profiles[0].choice.agent).toBe("claude");
    const custom = old(); custom.profiles[0].choice.model = "opus";
    const kept = initializeDefaults(custom);
    expect(kept.profiles[0].choice.model).toBe("opus");
    expect(kept.profiles[0]).toMatchObject({ provider_rule: "fixed", candidates: [], access: "default" });
  });
  it("mirrors the backend validation", () => {
    const capable = (agent: string) => agent !== "antigravity";
    const review = initializeDefaults(emptyCatalog()).profiles[0];
    expect(validRules(review, capable)).toBe(true);
    expect(validRules({ ...review, choice: { agent: "claude", model: "", effort: "" } }, capable)).toBe(false);
    expect(validRules({ ...review, candidates: [...review.candidates, { agent: "codex", model: "", effort: "" }] }, capable)).toBe(false);
    expect(validRules({ ...review, candidates: [...review.candidates, { agent: "antigravity", model: "", effort: "" }] }, capable)).toBe(false);
    expect(validRules({ ...review, mcp: ["server"] }, capable)).toBe(false);
    expect(validRules({ ...review, mcp: [] }, capable)).toBe(true);
    expect(validRules({ ...review, provider_rule: "fixed" }, capable)).toBe(false);
    expect(validRules({ ...review, provider_rule: "fixed", candidates: [] }, capable)).toBe(true);
  });
});
```

Run: `npx vitest run src/actions.test.ts` → FAIL (`validRules` missing, seed shape).

- [ ] **Step 7: Implement the TypeScript mirror**

In `src/actions.ts`, import `ProviderId` from `./types` and replace the types and `initializeDefaults`:

```ts
export type ProviderRule = "fixed" | "different_from_builder";
export type Access = "default" | "read_only";
export type Profile = {
  id: string; name: string; prompt: string; choice: Choice;
  /// `different_from_builder` starts with the first usable candidate that did not build the workspace; `choice` mirrors candidates[0].
  provider_rule: ProviderRule; candidates: Choice[]; access: Access;
  mcp: string[] | null; plugins: string[] | null; skills: string[];
  permission: "ask" | "auto"; watch: Watch | null;
};
export type Catalog = { defaults_initialized?: boolean; defaults_revision?: number; profiles: Profile[]; commands: Action[]; overrides: Record<string, Record<string, Profile>>; pr_action: string | null };

type Seed = { revision: number; profile: Profile; command: Action; previous: Profile[] };
/// Mirror of the backend's serde defaults for catalogs saved before the provider rule and access level.
function withDefaults(catalog: Catalog) {
  for (const p of [...catalog.profiles, ...Object.values(catalog.overrides).flatMap(map => Object.values(map))]) {
    p.provider_rule ??= "fixed"; p.candidates ??= []; p.access ??= "default";
  }
}
const same = (a: Profile, b: Profile) => {
  const shape = (p: Profile) => JSON.stringify([p.id, p.name, p.prompt, [p.choice.agent, p.choice.model, p.choice.effort],
    p.provider_rule, p.candidates.map(c => [c.agent, c.model, c.effort]), p.access, p.mcp, p.plugins, p.skills, p.permission,
    p.watch && [p.watch.interval_seconds, p.watch.comments, p.watch.ci, p.watch.max_turns]]);
  return shape(a) === shape(b);
};
/// Mirror of Catalog::initialize_defaults.
export function initializeDefaults(catalog: Catalog): Catalog {
  const seed = JSON.parse(defaultsText) as Seed;
  if (catalog.defaults_initialized && (catalog.defaults_revision ?? 0) >= seed.revision) return catalog;
  const next = structuredClone(catalog);
  withDefaults(next);
  for (const old of seed.previous) withDefaults({ ...emptyCatalog(), profiles: [old] });
  if (!next.defaults_initialized) {
    if (!next.profiles.some(p => p.id === seed.profile.id) && !next.commands.some(c => c.name === seed.command.name)) {
      next.profiles.push(seed.profile); next.commands.push(seed.command);
    }
  } else {
    const at = next.profiles.findIndex(p => seed.previous.some(old => same(old, p)));
    if (at >= 0) next.profiles[at] = seed.profile;
  }
  next.defaults_initialized = true;
  next.defaults_revision = seed.revision;
  return next;
}
/// Mirror of the backend's provider-rule and access validation, so the editor can explain what the backend would refuse.
export function validRules(p: Profile, readOnlyCapable: (agent: ProviderId) => boolean): boolean {
  const different = p.provider_rule === "different_from_builder";
  const agents = different ? p.candidates.map(c => c.agent) : [p.choice.agent];
  const first = p.candidates[0];
  const rule = different
    ? agents.length >= 1 && agents.length <= 3 && new Set(agents).size === agents.length
      && first.agent === p.choice.agent && first.model === p.choice.model && first.effort === p.choice.effort
    : p.candidates.length === 0;
  return rule && (p.access !== "read_only" || (agents.every(readOnlyCapable) && !p.mcp?.length));
}
```

- [ ] **Step 8: Keep the other TypeScript literals and the mock valid**

`src/action-settings.ts`: in the `profileEditor` default and in `delivery()`, add `provider_rule: "fixed", candidates: [], access: "default",` after `choice`.

`src/mock.ts` `actions_save`: import `validRules`, and extend the refusal condition with
`|| [...catalog.profiles, ...Object.values(catalog.overrides).flatMap(map => Object.values(map))].some(p => !validRules(p, agent => mockProviders().find(d => d.id === agent)?.capabilities.readOnlyProfile ?? false))`.
Move the provider array returned by `agents()` into `function mockProviders()` (same literal, with `readOnlyProfile` from Task 1) and make `agents()` return `{ providers: mockProviders() }` after its delay.

`e2e/actions.spec.ts` (second test): after `const profile = board.actions.profiles[0];` add `profile.provider_rule = "fixed"; profile.candidates = [];` so the scenario keeps exercising a fixed historical model.

- [ ] **Step 9: Run the checks**

Run: `npx vitest run src/actions.test.ts && npm run typecheck && cd src-tauri && cargo test --lib actions:: && cd .. && npx playwright test e2e/actions.spec.ts`
Expected: PASS.

- [ ] **Step 10: Commit**

```bash
git add src-tauri/src/state.rs src-tauri/src/actions.rs src-tauri/src/session.rs src/action-defaults.json src/actions.ts src/actions.test.ts src/action-settings.ts src/mock.ts e2e/actions.spec.ts
git commit -m "refactor(actions): add provider rule and access level to profiles" -m "Seed revision 2 makes the bundled review cross-family and read-only; an untouched revision 1 profile is upgraded once." -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Pick the reviewer at task start

**Files:**
- Create: `src-tauri/src/actions/reviewer.rs`
- Modify: `src-tauri/src/actions.rs` (module, `Run`, `resolve`, `action_start`, tests)
- Modify: `src-tauri/src/accounts.rs` (`signed_in`, test)
- Modify: `src-tauri/src/platform.rs:64-74, 91-96`
- Modify: `src-tauri/src/agents.rs` (`usable`)
- Modify: `src-tauri/src/session.rs` (test calling `resolve`)
- Modify: `src/actions.ts`, `src/actions.test.ts`, `src/mock.ts:1073-1103`

**Interfaces:**
- Consumes: Task 2 types.
- Produces: `actions::reviewer::{builders(&Workspace) -> Vec<ProviderId>, pick(&Profile, &[ProviderId], impl FnMut(ProviderId) -> bool) -> Pick, Pick { choice: Choice, same_family: bool }}`; `actions::resolve(c, project, id, pick: impl FnOnce(&Profile) -> reviewer::Pick, tools: impl FnOnce(ProviderId) -> session::ResolvedTools) -> Result<(Profile, bool), String>`; `Run.same_family: bool`; `agents::usable(ProviderId) -> bool`; `accounts::signed_in(ProviderId) -> bool`; TS `builders(ws)`, `pick(profile, builders, usable) -> { choice: Choice; same_family: boolean }`, `TaskRun.same_family?: boolean`.

- [ ] **Step 1: Write the failing selection tests**

Create `src-tauri/src/actions/reviewer.rs` with only the tests module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::{Access, Permission};
    use crate::state::Tab;
    use serde_json::json;
    use ProviderId::{Antigravity, Claude, Codex};

    fn choice(agent: ProviderId) -> Choice {
        Choice { agent, ..Default::default() }
    }

    fn review(candidates: &[ProviderId]) -> Profile {
        Profile {
            id: "review".into(),
            name: "Review".into(),
            prompt: "Review".into(),
            choice: choice(candidates[0]),
            provider_rule: ProviderRule::DifferentFromBuilder,
            candidates: candidates.iter().copied().map(choice).collect(),
            access: Access::ReadOnly,
            mcp: None,
            plugins: None,
            skills: vec![],
            permission: Permission::Ask,
            watch: None,
        }
    }

    fn picked(pick: Pick) -> (ProviderId, bool) {
        (pick.choice.agent, pick.same_family)
    }

    #[test]
    fn prefers_a_family_that_did_not_build() {
        let p = review(&[Codex, Claude]);
        assert_eq!(picked(pick(&p, &[Claude], |_| true)), (Codex, false));
        assert_eq!(picked(pick(&p, &[Codex], |_| true)), (Claude, false));
    }

    #[test]
    fn mixed_builders_fall_back_to_the_first_usable_candidate() {
        let p = review(&[Codex, Claude]);
        assert_eq!(picked(pick(&p, &[Claude, Codex], |_| true)), (Codex, true));
        let three = review(&[Codex, Claude, Antigravity]);
        assert_eq!(picked(pick(&three, &[Claude, Codex], |_| true)), (Antigravity, false));
    }

    #[test]
    fn uninstalled_and_signed_out_candidates_are_skipped() {
        let p = review(&[Codex, Claude]);
        // Codex is not installed: the other family is unavailable, so Claude reviews Claude.
        assert_eq!(picked(pick(&p, &[Claude], |agent| agent != Codex)), (Claude, true));
        // Claude is signed out: Codex reviews Codex.
        assert_eq!(picked(pick(&p, &[Codex], |agent| agent != Claude)), (Codex, true));
    }

    #[test]
    fn nothing_usable_keeps_the_first_candidate_for_the_spawn_to_report() {
        let p = review(&[Codex, Claude]);
        assert_eq!(picked(pick(&p, &[Claude], |_| false)), (Codex, false));
        assert_eq!(picked(pick(&p, &[Codex], |_| false)), (Codex, true));
    }

    #[test]
    fn fixed_profiles_never_probe_providers() {
        let mut p = review(&[Codex]);
        p.provider_rule = ProviderRule::Fixed;
        p.candidates.clear();
        p.choice.model = "gpt".into();
        let result = pick(&p, &[Codex], |_| panic!("fixed profiles do not probe providers"));
        assert_eq!(result, Pick { choice: p.choice.clone(), same_family: false });
    }

    #[test]
    fn availability_is_checked_lazily_once_per_candidate() {
        let p = review(&[Codex, Claude, Antigravity]);
        let mut calls = vec![];
        pick(&p, &[Claude], |agent| {
            calls.push(agent);
            true
        });
        assert_eq!(calls, [Codex]);
        calls.clear();
        pick(&p, &[Codex, Claude, Antigravity], |agent| {
            calls.push(agent);
            agent == Antigravity
        });
        assert_eq!(calls, [Codex, Claude, Antigravity]);
    }

    #[test]
    fn builders_are_ordinary_tabs_or_the_workspace_default() {
        let mut ws: Workspace = serde_json::from_value(json!({
            "id": "w", "title": "", "repo": "", "repo_name": "", "branch": "",
            "worktree": "", "stage": "", "agent": "codex"
        }))
        .unwrap();
        let tab = |id: &str, agent: Option<ProviderId>, task: bool| -> Tab {
            let mut tab: Tab = serde_json::from_value(json!({
                "id": id, "title": "", "status": "pronta", "note": null, "pending_prompt": null
            }))
            .unwrap();
            tab.choice = agent.map(choice);
            tab.task = task.then(|| {
                serde_json::from_value(json!({
                    "command": "review", "profile": review(&[Claude]), "paused": false,
                    "done": true, "turns": 0, "checked_at": 0, "error": null
                }))
                .unwrap()
            });
            tab
        };
        assert_eq!(builders(&ws), [Codex], "no tabs: the workspace default built it");
        ws.tabs = vec![tab("review", Some(Claude), true)];
        assert_eq!(builders(&ws), [Codex], "an earlier review does not build");
        ws.tabs.push(tab("inherits", None, false));
        ws.tabs.push(tab("override", Some(Claude), false));
        ws.tabs.push(tab("again", Some(Claude), false));
        assert_eq!(builders(&ws), [Codex, Claude]);
    }
}
```

In `src-tauri/src/actions.rs`, add `mod reviewer;` after the `use` lines and `pub use reviewer::Pick;`.

- [ ] **Step 2: Run them to verify they fail**

Run: `cd src-tauri && cargo test --lib actions::reviewer`
Expected: compile errors for `pick`, `builders`, `Pick`.

- [ ] **Step 3: Implement the selection**

At the top of `src-tauri/src/actions/reviewer.rs`:

```rust
//! Pick the provider a task starts with. Pure: availability arrives as a function, so builders,
//! installations and sign-ins combine in tests without CLIs or accounts.

use super::{Profile, ProviderRule};
use crate::state::{Choice, ProviderId, Workspace};

/// The provider a task starts with and whether it shares a builder's family.
#[derive(Clone, Debug, PartialEq)]
pub struct Pick {
    pub choice: Choice,
    pub same_family: bool,
}

/// Providers of the workspace's ordinary tabs: each tab's own choice, or the workspace default it
/// inherits. Task tabs do not build. A workspace without ordinary tabs counts its default provider.
pub fn builders(ws: &Workspace) -> Vec<ProviderId> {
    let mut found = Vec::new();
    for tab in ws.tabs.iter().filter(|tab| tab.task.is_none()) {
        let agent = tab.choice.as_ref().map_or(ws.agent, |choice| choice.agent);
        if !found.contains(&agent) {
            found.push(agent);
        }
    }
    if found.is_empty() {
        found.push(ws.agent);
    }
    found
}

/// A fixed profile keeps its choice. Otherwise take the first usable candidate outside the
/// builders, then the first usable one, then the first candidate, so the spawn reports why it
/// cannot run instead of this rule refusing the review. `usable` runs at most once per candidate
/// and only when needed, because checking a provider may start a process.
pub fn pick(
    profile: &Profile,
    builders: &[ProviderId],
    mut usable: impl FnMut(ProviderId) -> bool,
) -> Pick {
    let candidates = &profile.candidates;
    if profile.provider_rule == ProviderRule::Fixed || candidates.is_empty() {
        return Pick {
            choice: profile.choice.clone(),
            same_family: false,
        };
    }
    let mut known: Vec<Option<bool>> = vec![None; candidates.len()];
    let mut ready =
        |index: usize| *known[index].get_or_insert_with(|| usable(candidates[index].agent));
    let index = (0..candidates.len())
        .find(|&index| !builders.contains(&candidates[index].agent) && ready(index))
        .or_else(|| (0..candidates.len()).find(|&index| ready(index)))
        .unwrap_or(0);
    let choice = candidates[index].clone();
    Pick {
        same_family: builders.contains(&choice.agent),
        choice,
    }
}
```

Run: `cd src-tauri && cargo test --lib actions::reviewer` → PASS.

- [ ] **Step 4: Write the failing freezing test**

In `src-tauri/src/actions.rs` tests:

```rust
    #[test]
    fn resolution_picks_the_provider_before_tools_and_freezes_read_only_mcp() {
        let mut review = profile();
        review.watch = None;
        review.provider_rule = ProviderRule::DifferentFromBuilder;
        review.choice = Choice { agent: ProviderId::Codex, ..Default::default() };
        review.candidates = vec![review.choice.clone(), Choice::default()];
        review.access = Access::ReadOnly;
        let catalog = Catalog { profiles: vec![review], ..Default::default() };
        let (frozen, same_family) = resolve(
            &catalog,
            "",
            "reviewer",
            |p| reviewer::pick(p, &[ProviderId::Codex], |_| true),
            |agent| {
                assert_eq!(agent, ProviderId::Claude, "tools resolve for the picked provider");
                session::ResolvedTools { mcp: Some(vec!["hub".into()]), ..Default::default() }
            },
        )
        .unwrap();
        assert_eq!(frozen.choice.agent, ProviderId::Claude);
        assert!(!same_family);
        assert_eq!(frozen.provider_rule, ProviderRule::Fixed);
        assert!(frozen.candidates.is_empty());
        assert_eq!(frozen.mcp, Some(vec![]));
        assert!(validate_profile(&frozen).is_ok());
    }
```

Add `same_family: false,` to the two `Run` literals in the tests. In `src-tauri/src/session.rs` `task_launch_uses_project_override_and_freezes_tools_and_model`, change the call to:

```rust
        let (profile, _) = crate::actions::resolve(
            &catalog,
            &ws.project,
            "review",
            |p| crate::actions::Pick { choice: p.choice.clone(), same_family: false },
            |agent| {
                assert_eq!(agent, ProviderId::Claude);
                resolved
            },
        )
        .unwrap();
```

Run: `cd src-tauri && cargo test --lib actions::tests::resolution_picks` → FAIL (arity).

- [ ] **Step 5: Implement freezing and wire `action_start`**

Replace `resolve` in `src-tauri/src/actions.rs`:

```rust
/// Freeze a profile for one execution and report whether its provider shares a builder's family.
/// The project override replaces the global profile; `pick` chooses the provider, frozen as a fixed
/// choice; tools resolve for that provider, since the MCP base differs between providers.
pub fn resolve(
    c: &Catalog,
    project: &str,
    id: &str,
    pick: impl FnOnce(&Profile) -> Pick,
    tools: impl FnOnce(ProviderId) -> session::ResolvedTools,
) -> Result<(Profile, bool), String> {
    let mut p = c
        .overrides
        .get(project)
        .and_then(|map| map.get(id))
        .or_else(|| c.profiles.iter().find(|p| p.id == id))
        .cloned()
        .ok_or_else(|| i18n::t("err.actions.missing"))?;
    let picked = pick(&p);
    p.choice = picked.choice;
    p.provider_rule = ProviderRule::Fixed;
    p.candidates.clear();
    let resolved = tools(p.choice.agent);
    // A task freezes the tools it starts with, so the caller resolves the layers once and an axis
    // the profile leaves unset inherits that resolved global and workspace selection.
    if p.access == Access::ReadOnly {
        // MCP tools run outside both providers' read-only envelopes.
        p.mcp = Some(vec![]);
    } else if p.mcp.is_none() {
        p.mcp = resolved.mcp.clone();
    }
    if p.plugins.is_none() {
        // Standalone skills ride the plugin pipeline, so the frozen set carries both axes.
        p.plugins = resolved.plugin_packages();
    }
    Ok((p, picked.same_family))
}
```

Add to `Run` after `prs`:

```rust
    /// The rule found no usable candidate outside the builders' families. Absent in older tasks.
    #[serde(default)]
    pub same_family: bool,
```

In `action_start`, replace the `let profile = resolve(...)` call with:

```rust
        let builders = reviewer::builders(&ws);
        let (profile, same_family) = resolve(
            &actions,
            &ws.project,
            a.profile.as_deref().unwrap_or(""),
            |p| reviewer::pick(p, &builders, crate::agents::usable),
            |agent| session::resolve_workspace_tools(&global, &trust, &ws, agent),
        )?;
```

and add `same_family,` to the `Run` literal.

- [ ] **Step 6: Implement availability**

`src-tauri/src/platform.rs`: delete `#[cfg(not(target_os = "macos"))]` above `pub fn has` and above the `finds_programs_on_path_only` test (the adopted login-shell PATH makes it valid on macOS).

`src-tauri/src/accounts.rs`, in `impl Registry` next to `find`:

```rust
    /// The active account's last observed sign-in. Usage probes refresh it, so it can lag a login
    /// or logout by one probe interval.
    fn signed_in(&self, provider: ProviderId) -> bool {
        self.active
            .get(key(provider))
            .and_then(|id| self.find(id).ok())
            .is_some_and(|account| account.identity.connected)
    }
```

and after `pub fn active`:

```rust
pub fn signed_in(provider: ProviderId) -> bool {
    lock(registry())
        .as_ref()
        .is_ok_and(|data| data.signed_in(provider))
}
```

with this test in its `tests` module:

```rust
    #[test]
    fn signed_in_reads_the_active_accounts_last_identity() {
        let mut registry = Registry::default();
        assert!(!registry.signed_in(ProviderId::Codex), "never probed");
        let codex = registry.accounts.iter_mut().find(|a| a.id == "codex").unwrap();
        codex.identity.connected = true;
        assert!(registry.signed_in(ProviderId::Codex));
        assert!(!registry.signed_in(ProviderId::Claude));
        assert!(!registry.signed_in(ProviderId::Antigravity), "no active account");
        registry.active.insert("codex".into(), "missing".into());
        assert!(!registry.signed_in(ProviderId::Codex));
    }
```

`src-tauri/src/agents.rs`, after `agents()`:

```rust
/// A provider can take a task now when its CLI is on the adopted PATH and its active account is
/// signed in. Antigravity's identity is never probed, so its version check and an attached account
/// stand for both.
pub fn usable(id: ProviderId) -> bool {
    match id {
        ProviderId::Claude => crate::platform::has("claude") && crate::accounts::signed_in(id),
        ProviderId::Codex => crate::platform::has("codex") && crate::accounts::signed_in(id),
        ProviderId::Antigravity => {
            crate::accounts::active(id).is_ok() && crate::antigravity::installed()
        }
        ProviderId::RetiredGemini => false,
    }
}
```

- [ ] **Step 7: Run the Rust tests**

Run: `cd src-tauri && cargo test --lib actions:: accounts::tests::signed_in platform::tests && cargo test --lib session::tests::task_launch`
Expected: PASS.

- [ ] **Step 8: Write the failing TypeScript mirror tests**

Add to `src/actions.test.ts` (import `builders, pick`):

```ts
describe("reviewer selection mirror", () => {
  const review = () => initializeDefaults(emptyCatalog()).profiles[0];
  it("matches the backend cases", () => {
    const p = review();
    expect(pick(p, ["claude"], () => true)).toEqual({ choice: { agent: "codex", model: "", effort: "" }, same_family: false });
    expect(pick(p, ["codex"], () => true).choice.agent).toBe("claude");
    expect(pick(p, ["claude", "codex"], () => true)).toMatchObject({ choice: { agent: "codex" }, same_family: true });
    expect(pick(p, ["claude"], agent => agent !== "codex")).toMatchObject({ choice: { agent: "claude" }, same_family: true });
    expect(pick(p, ["claude"], () => false)).toMatchObject({ choice: { agent: "codex" }, same_family: false });
    const fixed = { ...p, provider_rule: "fixed" as const, candidates: [] };
    expect(pick(fixed, ["codex"], () => { throw new Error("fixed profiles do not probe providers"); }).choice).toEqual(fixed.choice);
  });
  it("counts ordinary tabs or the workspace default as builders", () => {
    const tab = (id: string, agent?: "claude" | "codex", task = false) => ({ id, title: "", status: "pronta" as const, note: null, tokens: null,
      choice: agent ? { agent, model: "", effort: "" } : null, task: task ? { command: "review" } as TaskRun : null });
    expect(builders({ agent: "codex", tabs: [] })).toEqual(["codex"]);
    expect(builders({ agent: "codex", tabs: [tab("review", "claude", true)] })).toEqual(["codex"]);
    expect(builders({ agent: "codex", tabs: [tab("a"), tab("b", "claude"), tab("c", "claude")] })).toEqual(["codex", "claude"]);
  });
});
```

Run: `npx vitest run src/actions.test.ts` → FAIL.

- [ ] **Step 9: Implement the TypeScript mirror and the mock**

In `src/actions.ts` (import `Workspace` from `./types`):

```ts
export type TaskRun = {
  command: string; profile: Profile; paused: boolean; done: boolean; turns: number;
  checked_at: number; error: string | null; seen: Record<string, string>; prs: Record<string, number>;
  /// No usable candidate outside the builders' families existed; absent in older tasks.
  same_family?: boolean;
};
/// Mirror of reviewer::builders.
export function builders(ws: Pick<Workspace, "agent" | "tabs">): ProviderId[] {
  const found: ProviderId[] = [];
  for (const tab of ws.tabs) {
    if (tab.task) continue;
    const agent = tab.choice?.agent ?? ws.agent;
    if (!found.includes(agent)) found.push(agent);
  }
  return found.length ? found : [ws.agent];
}
/// Mirror of reviewer::pick.
export function pick(profile: Profile, builders: ProviderId[], usable: (agent: ProviderId) => boolean): { choice: Choice; same_family: boolean } {
  const candidates = profile.candidates;
  if (profile.provider_rule !== "different_from_builder" || !candidates.length) return { choice: { ...profile.choice }, same_family: false };
  const known = new Map<ProviderId, boolean>();
  const ready = (agent: ProviderId) => { if (!known.has(agent)) known.set(agent, usable(agent)); return known.get(agent)!; };
  const choice = candidates.find(c => !builders.includes(c.agent) && ready(c.agent)) ?? candidates.find(c => ready(c.agent)) ?? candidates[0];
  return { choice: { ...choice }, same_family: builders.includes(choice.agent) };
}
```

In `src/mock.ts` add, next to `mockProviders()`:

```ts
/// Mirror of agents::usable over the mock's installations and accounts; attached Antigravity accounts are never probed.
function mockUsable(agent: ProviderId): boolean {
  const provider = mockProviders().find(p => p.id === agent);
  const account = mockAccounts.accounts.find(a => a.id === mockAccounts.active[agent]);
  return !!provider?.installed && !!account && (account.connected || provider.authMethods.some(m => m.kind === "external"));
}
```

and in `action_start` replace `const profile = structuredClone(...) as Profile;` through the MCP line with:

```ts
    const source = structuredClone(catalog.overrides[workspace.project]?.[action.profile] ?? catalog.profiles.find(p => p.id === action.profile)) as Profile;
    // Mirror of resolve: the provider is picked first because tool resolution depends on it.
    const picked = pick(source, builders(workspace), mockUsable);
    const profile: Profile = { ...source, choice: picked.choice, provider_rule: "fixed", candidates: [] };
```

keep the existing `toolLayers`/`axis`/plugins lines, change the MCP line to
`if (profile.access === "read_only") profile.mcp = []; else profile.mcp ??= axis(l.global.mcp, project.mcp, l.own.mcp, l.base, l.mcpUniverse);`
and add `same_family: picked.same_family` to the `task` literal. Import `builders, pick` from `./actions` and `ProviderId` from `./types` if missing.

- [ ] **Step 10: Run the checks**

Run: `npx vitest run src/actions.test.ts && npm run typecheck && cd src-tauri && cargo test --lib && cargo clippy --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 11: Commit**

```bash
git add src-tauri/src/actions.rs src-tauri/src/actions/reviewer.rs src-tauri/src/accounts.rs src-tauri/src/platform.rs src-tauri/src/agents.rs src-tauri/src/session.rs src/actions.ts src/actions.test.ts src/mock.ts
git commit -m "feat(review): review with a provider family other than the builder's" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Claude enforces read-only access

**Files:**
- Modify: `src-tauri/src/session.rs:911-943, 1305-1320` and its tests
- Modify: `src-tauri/src/claude.rs:196-297` and `launch_tests`
- Create: `src-tauri/src/claude/fixtures/read-only.ndjson`, `src-tauri/src/claude/fixtures/README.md`

**Interfaces:**
- Consumes: `actions::Access` (Task 2), frozen `run.profile.access`.
- Produces: `session::Launch { access: crate::actions::Access, .. }` (serde default); `claude::command(args, worktree, launch, profile) -> Result<Command, String>`.

- [ ] **Step 1: Thread the access through task launches (test first)**

Add to the `session.rs` tests, next to `task_launch_uses_project_override_and_freezes_tools_and_model`:

```rust
    #[test]
    fn task_launch_carries_the_frozen_access() {
        let mut ws = bare();
        let mut task = tab("task", None);
        task.task = Some(
            serde_json::from_value(serde_json::json!({
                "command": "review", "profile": {
                    "id": "review", "name": "Review", "prompt": "Review",
                    "choice": {"agent": "codex", "model": "", "effort": ""},
                    "access": "read_only", "mcp": [], "plugins": null,
                    "skills": [], "permission": "ask", "watch": null
                }, "paused": false, "done": false, "turns": 0,
                "checked_at": 0, "error": null
            }))
            .unwrap(),
        );
        ws.tabs.push(task);
        let launch = ws.launch_of("task", &ResolvedTools::default());
        assert_eq!(launch.access, crate::actions::Access::ReadOnly);
        assert_eq!(launch.mcp, Some(vec![]));
        assert_eq!(ws.launch(&ResolvedTools::default()).access, crate::actions::Access::Default);
    }
```

Run: `cd src-tauri && cargo test --lib session::tests::task_launch_carries` → FAIL (`no field access`).

- [ ] **Step 2: Implement `Launch.access`**

In `Launch`, after `permission`:

```rust
    /// Read-only access removes write tools in the adapter; only task profiles set it.
    #[serde(default)]
    pub access: crate::actions::Access,
```

In `launch_of`'s task branch add `access: run.profile.access,`.

Run: `cd src-tauri && cargo test --lib session::tests::task_launch` → PASS.

- [ ] **Step 3: Write the failing Claude argument tests**

In `claude.rs` `launch_tests` add:

```rust
    fn read_only() -> Launch {
        Launch {
            access: crate::actions::Access::ReadOnly,
            permission: Some(crate::actions::Permission::Auto),
            mcp: Some(vec![]),
            ..Default::default()
        }
    }

    #[test]
    fn read_only_restricts_tools_settings_and_mcp_without_prompts() {
        let args = launch_args("id", false, &read_only(), work()).unwrap();
        let has = |pair: [&str; 2]| args.windows(2).any(|w| w[0] == pair[0] && w[1] == pair[1]);
        assert!(args.contains(&"--restricted".to_string()));
        assert!(has(["--permission-mode", "dontAsk"]));
        assert!(has(["--tools", "Read,Grep,Glob,Bash,Skill,Task"]));
        assert!(has(["--add-dir", "/prometeu-launch-test"]));
        assert!(args.contains(&"--strict-mcp-config".to_string()));
        assert!(has(["--mcp-config", r#"{"mcpServers":{}}"#]));
        for flag in ["--dangerously-skip-permissions", "--allow-dangerously-skip-permissions"] {
            assert!(!args.contains(&flag.to_string()), "{flag}");
        }
    }

    #[test]
    fn read_only_refuses_plan_mode_and_mcp_servers() {
        let plan = Launch { plan: true, ..read_only() };
        let servers = Launch { mcp: Some(vec!["server".into()]), ..read_only() };
        assert!(launch_args("id", false, &plan, work()).is_err());
        assert!(launch_args("id", false, &servers, work()).is_err());
    }

    #[test]
    fn read_only_memory_survives_inherited_claude_variables() {
        let key = "CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD";
        std::env::set_var(key, "0");
        let profile = crate::accounts::Profile {
            id: "claude".into(),
            provider: ProviderId::Claude,
            home: std::env::temp_dir(),
            managed: false,
            revision: 0,
        };
        let cmd = command(vec![], work(), &read_only(), &profile).unwrap();
        let ordinary = command(vec![], work(), &launch("", "", false), &profile).unwrap();
        std::env::remove_var(key);
        let value = |cmd: &Command| {
            cmd.get_envs()
                .find(|(k, _)| *k == key)
                .and_then(|(_, v)| v.map(|v| v.to_string_lossy().into_owned()))
        };
        assert_eq!(value(&cmd).as_deref(), Some("1"));
        assert_eq!(value(&ordinary), None, "ordinary sessions remove the inherited value");
    }
```

Keep `plan_mode_does_not_enable_permission_bypass` and `task_permissions_and_instructions_reach_claude` unchanged: they pin the default branches.

Run: `cd src-tauri && cargo test --lib claude::launch_tests` → FAIL.

- [ ] **Step 4: Implement the Claude materialization**

In `claude.rs`, above `launch_args`:

```rust
/// Tools a read-only session keeps. Write tools are absent in the session and its subagents; web
/// tools are absent because they reach the network.
const READ_ONLY_TOOLS: &str = "Read,Grep,Glob,Bash,Skill,Task";
/// Restricted mode skips settings files, so the project's CLAUDE.md loads only through the added
/// worktree with this variable (verified with Claude 2.1.283).
const READ_ONLY_MEMORY: (&str, &str) = ("CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD", "1");
/// An inline empty configuration loads no MCP server, not even account connectors, without the
/// connector lookup a hub selection needs.
const NO_MCP: &str = r#"{"mcpServers":{}}"#;
```

In `launch_args`, before the `if launch.plan {` block:

```rust
    let read_only = launch.access == crate::actions::Access::ReadOnly;
    if read_only && (launch.plan || launch.mcp.as_ref().is_some_and(|ids| !ids.is_empty())) {
        // The core never freezes these together; refuse rather than drop a requested behavior.
        return Err(i18n::t("err.actions.invalid"));
    }
```

and turn the permission block into:

```rust
    if read_only {
        // Restricted mode ignores user, project and local settings files, so no allow rule,
        // project skill or setting widens this envelope, and the CLI refuses bypassPermissions.
        // dontAsk denies what the CLI does not classify as read-only instead of prompting.
        args.extend(
            ["--restricted", "--permission-mode", "dontAsk", "--tools", READ_ONLY_TOOLS, "--add-dir"]
                .map(String::from),
        );
        args.push(worktree.display().to_string());
    } else if launch.plan {
```

(the existing `plan` and `permission` branches follow unchanged). Replace the MCP block with:

```rust
    if read_only {
        args.extend(["--strict-mcp-config", "--mcp-config", NO_MCP].map(String::from));
    } else if let Some(path) = crate::mcp::config_for(id, launch.mcp.as_ref(), worktree)? {
        args.extend([
            "--mcp-config".into(),
            path.display().to_string(),
            "--strict-mcp-config".into(),
        ]);
    }
```

keeping the existing ADR 0046 comment above it. Add after `launch_args`:

```rust
/// Build the process with the account's environment. The read-only variable goes after
/// `account_env`, which strips every inherited CLAUDE* variable.
fn command(
    args: Vec<String>,
    worktree: &Path,
    launch: &crate::session::Launch,
    profile: &accounts::Profile,
) -> Result<Command, String> {
    let mut cmd = Command::new("claude");
    cmd.args(args).current_dir(worktree);
    profile.apply(&mut cmd)?;
    if launch.access == crate::actions::Access::ReadOnly {
        cmd.env(READ_ONLY_MEMORY.0, READ_ONLY_MEMORY.1);
    }
    Ok(cmd)
}
```

and in `spawn`, replace `let mut cmd = Command::new("claude"); cmd.args(args).current_dir(worktree); profile.apply(&mut cmd)?;` with `let cmd = command(args, worktree, launch, &profile)?;`.

Run: `cd src-tauri && cargo test --lib claude::` → PASS.

- [ ] **Step 5: Record the Claude fixture**

Capture a real session (costs one short Haiku turn) in a scratch repo with an uncommitted change:

```bash
rm -rf /tmp/prometeu-ro && mkdir -p /tmp/prometeu-ro && cd /tmp/prometeu-ro && git init -q -b main && printf 'fn main() {}\n' > main.rs && git add . && git -c commit.gpgsign=false -c user.email=f@f -c user.name=f commit -qm init && printf 'fn main() { println!("x"); }\n' > main.rs
printf '%s\n' '{"type":"user","message":{"role":"user","content":"This is a sandbox verification. Do these steps in order, one tool call per step, and report each outcome in one line. Do not ask me anything; if a step is refused or a tool is unavailable, continue. 1) Run `git diff --stat`. 2) Create the file created-by-write.txt with content x using a file-writing tool. 3) Run `touch created-by-touch.txt`."}}' \
 | CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD=1 claude -p --input-format stream-json --output-format stream-json --verbose --permission-prompt-tool stdio --session-id 00000000-0000-4000-8000-000000000141 --model haiku --restricted --permission-mode dontAsk --tools Read,Grep,Glob,Bash,Skill,Task --add-dir /tmp/prometeu-ro --strict-mcp-config --mcp-config '{"mcpServers":{}}' > /tmp/prometeu-ro.ndjson
ls /tmp/prometeu-ro   # expect only main.rs: nothing was created
```

Sanitize into the fixture (drops partial-message and account lines, keeps a minimal `init`, replaces paths):

```bash
python3 - /tmp/prometeu-ro.ndjson src-tauri/src/claude/fixtures/read-only.ndjson <<'EOF'
import json, sys
src, dst = sys.argv[1], sys.argv[2]
out = []
for line in open(src):
    v = json.loads(line)
    kind = v.get("type")
    if kind == "system" and v.get("subtype") == "init":
        v = {k: v[k] for k in ("type", "subtype", "cwd", "session_id", "tools", "mcp_servers", "permissionMode") if k in v}
    elif kind not in ("assistant", "user", "result"):
        continue
    text = json.dumps(v, ensure_ascii=False)
    for path in ("/private/tmp/prometeu-ro", "/tmp/prometeu-ro"):
        text = text.replace(path, "/fixture/worktree")
    out.append(text)
open(dst, "w").write("\n".join(out) + "\n")
EOF
grep -c . src-tauri/src/claude/fixtures/read-only.ndjson && grep -ci "email\|@example\|kaique" src-tauri/src/claude/fixtures/read-only.ndjson || true
```

Expected: a few dozen lines, zero personal matches. If the adapter needs the dropped `stream_event` lines to emit tool events (Step 6 fails on missing `tool.completed`), keep `stream_event` lines too and re-run the same replacement.

Create `src-tauri/src/claude/fixtures/README.md`:

```md
# Claude stream fixtures

`read-only.ndjson` was recorded on 2026-09-28 from Claude Code 2.1.283 with the
read-only task flags (`--restricted --permission-mode dontAsk --tools
Read,Grep,Glob,Bash,Skill,Task --add-dir`, an inline empty MCP configuration
and `CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD=1`) in an empty scratch
repository with one uncommitted change. The prompt asked for `git diff --stat`,
a file write and `touch`. `git diff` ran; with Write absent the model tried to
write through Bash, and both commands were denied without a permission request;
no file was created. Only the initialization record was minimized, partial
message events and other system records were dropped, and paths were replaced
with `/fixture/worktree`.
```

- [ ] **Step 6: Prove the translation (test)**

In `claude.rs` tests (outside `launch_tests`, next to the other adapter tests):

```rust
    /// A read-only recording: reads run, writes are denied, and no permission request reaches V1.
    #[test]
    fn read_only_recording_denies_writes_without_requests() {
        let mut adapter = Adapter { fresh: true, ..Adapter::default() };
        let events: Vec<Value> = include_str!("claude/fixtures/read-only.ndjson")
            .lines()
            .flat_map(|line| adapter.translate_line(line))
            .map(|line| serde_json::from_str(&line).unwrap())
            .collect();
        assert!(events.iter().all(|e| e["type"] != "request.opened"));
        let results: Vec<&Value> = events.iter().filter(|e| e["type"] == "tool.completed").collect();
        assert!(results.iter().any(|e| e["error"] == false
            && e["output"].as_str().is_some_and(|out| out.contains("main.rs"))));
        assert!(results.iter().filter(|e| e["error"] == true).count() >= 2);
        assert!(events.iter().any(|e| e["type"] == "turn.completed"));
    }
```

Run: `cd src-tauri && cargo test --lib claude::` → PASS.

- [ ] **Step 7: Commit**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all
git add src-tauri/src/session.rs src-tauri/src/claude.rs src-tauri/src/claude/fixtures
git commit -m "feat(review): run read-only Claude tasks without write access" -m "Restricted dontAsk mode keeps read tools only, ignores settings files that could widen permissions and loads no MCP server; the recording shows denied writes without prompts." -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Codex enforces read-only access; Antigravity refuses it

**Files:**
- Modify: `src-tauri/src/codex.rs:25-114, 665-686` and tests (`link()` at ~1427)
- Modify: `src-tauri/src/codex/contract.rs:5-16`
- Create: `src-tauri/src/codex/fixtures/read-only.ndjson`, `src-tauri/src/codex/fixtures/README.md`
- Modify: `src-tauri/src/antigravity.rs:179-190` and tests

**Interfaces:**
- Consumes: `Launch.access` (Task 4).
- Produces: `codex::Start { access: crate::actions::Access, .. }`.

- [ ] **Step 1: Write the failing Codex tests**

In `codex.rs` tests, add `access: Default::default(),` to the `Start` literal in `link()`, then:

```rust
    #[test]
    fn read_only_tasks_open_and_resume_in_the_read_only_sandbox() {
        for resume in [None, Some("previous")] {
            let (mut link, out) = link(resume);
            link.start.access = crate::actions::Access::ReadOnly;
            link.start.permission = Some(crate::actions::Permission::Ask);
            out.take();
            link.open_thread();
            let method = if resume.is_some() { "thread/resume" } else { "thread/start" };
            let sent = out.take();
            let message = sent.iter().find(|v| v["method"] == method).unwrap();
            assert_eq!(message["params"]["sandbox"], "read-only");
            assert_eq!(message["params"]["approvalPolicy"], "never");
        }
    }

    #[test]
    fn ordinary_sessions_keep_full_access() {
        let (mut link, out) = link(None);
        out.take();
        link.open_thread();
        let sent = out.take();
        let message = sent.iter().find(|v| v["method"] == "thread/start").unwrap();
        assert_eq!(message["params"]["sandbox"], "danger-full-access");
    }
```

In `codex/contract.rs` add `access: Default::default(),` to its `Start` literal.

Run: `cd src-tauri && cargo test --lib codex::tests::read_only` → FAIL.

- [ ] **Step 2: Implement the Codex materialization**

`Start`: add

```rust
    /// Read-only tasks run in the operating system's read-only sandbox without approvals.
    pub access: crate::actions::Access,
```

`spawn`: before building `cmd`,

```rust
    let read_only = launch.access == crate::actions::Access::ReadOnly;
    if read_only && launch.mcp.as_ref().is_some_and(|ids| !ids.is_empty()) {
        // The core never freezes these together; refuse rather than drop a requested server.
        return Err(i18n::t("err.actions.invalid"));
    }
    // Read-only replaces the whole MCP table, including servers from ~/.codex/config.toml.
    let no_mcp = Vec::new();
    let mcp = if read_only { Some(&no_mcp) } else { launch.mcp.as_ref() };
```

and pass `mcp` instead of `launch.mcp.as_ref()` to `crate::mcp::codex_config`; add `access: launch.access,` to the `Start` literal.

`open_thread`:

```rust
        let read_only = self.start.access == crate::actions::Access::ReadOnly;
        let mut params = json!({
            "cwd": self.start.cwd,
            // The read-only sandbox blocks writes and network for commands; nothing prompts.
            "approvalPolicy": if read_only {
                "never"
            } else if self.start.permission == Some(crate::actions::Permission::Ask) {
                "untrusted"
            } else {
                "never"
            },
            "sandbox": if read_only { "read-only" } else { "danger-full-access" },
        });
```

Update the module doc comment's "Default sessions use approvalPolicy never and an unrestricted sandbox." to add "; read-only tasks use the read-only sandbox."

Run: `cd src-tauri && cargo test --lib codex::` → PASS.

- [ ] **Step 3: Record the Codex fixture**

Reset the scratch repo from Task 4 (`cd /tmp/prometeu-ro && git checkout -q main.rs && printf 'fn main() { println!("x"); }\n' > main.rs`) and record one real turn with this probe, which opens the thread exactly as the adapter does and declines any server request it receives:

```bash
cat > /tmp/prometeu-ro-codex.py <<'EOF'
import json, subprocess, sys, threading
repo, out = "/tmp/prometeu-ro", "/tmp/prometeu-ro-codex.ndjson"
prompt = ("This is a sandbox verification. Execute each step for real even if you expect it to fail, and report "
          "the exact exit code and error text of each. Do not ask me anything. 1) Run exactly: git diff --stat "
          "2) Run exactly: touch created-by-touch.txt 3) Use your file editing tool (apply a patch) to add a new "
          "file created-by-patch.txt containing x.")
proc = subprocess.Popen(["codex", "app-server", "--enable", "default_mode_request_user_input", "-c",
                         "suppress_unstable_features_warning=true"], cwd=repo, stdin=subprocess.PIPE,
                        stdout=subprocess.PIPE, text=True, bufsize=1)
threading.Timer(300, proc.kill).start()
ids = iter(range(1, 100))
def send(obj): proc.stdin.write(json.dumps(obj) + "\n"); proc.stdin.flush()
def call(method, params):
    n = next(ids); send({"jsonrpc": "2.0", "id": n, "method": method, "params": params}); return n
init = call("initialize", {"clientInfo": {"name": "prometeu-fixture", "title": "Fixture", "version": "0"},
                           "capabilities": {"experimentalApi": True}})
thread, requests = None, 0
with open(out, "w") as log:
    for line in proc.stdout:
        log.write(line)
        v = json.loads(line)
        if v.get("id") == init and "method" not in v:
            send({"jsonrpc": "2.0", "method": "initialized", "params": {}})
            thread = call("thread/start", {"cwd": repo, "approvalPolicy": "never", "sandbox": "read-only"})
        elif v.get("id") == thread and "method" not in v:
            print("sandbox:", v["result"].get("sandbox"), "approval:", v["result"].get("approvalPolicy"))
            call("turn/start", {"threadId": v["result"]["thread"]["id"],
                                "input": [{"type": "text", "text": prompt, "text_elements": []}]})
        elif "method" in v and "id" in v:
            requests += 1
            send({"jsonrpc": "2.0", "id": v["id"], "result": {"decision": "decline"}})
        elif v.get("method") == "turn/completed":
            break
proc.kill()
print("server requests:", requests)
EOF
python3 /tmp/prometeu-ro-codex.py && ls /tmp/prometeu-ro
```

Expected: `sandbox: {'type': 'readOnly', 'networkAccess': False} approval: never`, `server requests: 0`, and `ls` lists only `main.rs`.

Sanitize:

```bash
python3 - /tmp/prometeu-ro-codex.ndjson src-tauri/src/codex/fixtures/read-only.ndjson <<'EOF'
import json, sys
src, dst = sys.argv[1], sys.argv[2]
keep = {"turn/started", "item/started", "item/completed", "thread/tokenUsage/updated", "turn/completed"}
thread, out = None, []
for line in open(src):
    v = json.loads(line)
    thread = thread or (v.get("result") or {}).get("thread", {}).get("id")
    if v.get("method") not in keep:
        continue
    text = json.dumps(v, ensure_ascii=False)
    for path in ("/private/tmp/prometeu-ro", "/tmp/prometeu-ro"):
        text = text.replace(path, "/wt")
    out.append(text.replace(thread, "t-1") if thread else text)
open(dst, "w").write("\n".join(out) + "\n")
EOF
```

Create `src-tauri/src/codex/fixtures/README.md`:

```md
# Codex fixtures

`process-peer.mjs` simulates an app-server process for lifecycle tests.

`read-only.ndjson` was recorded on 2026-09-28 from codex-cli 0.154.0 through
`app-server`, after `thread/start` with `sandbox: "read-only"` and
`approvalPolicy: "never"`; the response echoed
`{"type":"readOnly","networkAccess":false}`. The prompt asked for
`git diff --stat`, `touch` and a patch adding a file. `git diff` ran; the write
and the patch failed in the sandbox with no server request, and no file was
created. Codex 0.154.0 does not report the sandbox-denied command and patch as
items; the agent's final message reports them. Only turn and item notifications
were kept; the thread identity became `t-1` and paths became `/wt`.
```

- [ ] **Step 4: Prove the translation (test)**

```rust
    /// A read-only recording: the read runs and no approval request reaches the app.
    #[test]
    fn read_only_recording_has_no_approval_requests() {
        let (mut link, out) = link(None);
        opened(&mut link, &out);
        let events: Vec<Value> = include_str!("codex/fixtures/read-only.ndjson")
            .lines()
            .flat_map(|line| link.on_line(line))
            .collect();
        assert!(events.iter().all(|e| e["type"] != "request.opened"));
        assert!(events.iter().any(|e| e["type"] == "tool.completed" && e["error"] == false));
        assert!(events.iter().any(|e| e["type"] == "turn.completed"));
        assert!(out.take().iter().all(|m| m.get("result").is_none()), "no server request was answered");
    }
```

Run: `cd src-tauri && cargo test --lib codex::tests::read_only_recording` → PASS.

- [ ] **Step 5: Antigravity refuses read-only launches (test first)**

At the end of `launch_preserves_native_resume_and_permission_defaults` in `antigravity.rs`:

```rust
        launch.mcp = None;
        launch.access = crate::actions::Access::ReadOnly;
        assert!(launch_args(None, worktree, &launch).is_err());
```

Run → FAIL. Then in `launch_args`, change the first condition to
`if launch.plan || launch.access == crate::actions::Access::ReadOnly || [&launch.mcp, ...]` and run → PASS.

- [ ] **Step 6: Run the adapter suites and commit**

Run: `cd src-tauri && cargo fmt --all && cargo test --lib codex:: antigravity:: && cargo clippy --all-targets -- -D warnings`
Expected: PASS.

```bash
git add src-tauri/src/codex.rs src-tauri/src/codex/contract.rs src-tauri/src/codex/fixtures src-tauri/src/antigravity.rs
git commit -m "feat(review): run read-only Codex tasks in the read-only sandbox" -m "Antigravity cannot enforce read-only access and refuses it at the adapter." -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Profile editor for the provider rule and access level

**Files:**
- Modify: `src/model-picker.ts:8-20`
- Modify: `src/action-settings.ts` (imports, `settingsRows` profile cards, `selection`, `profileEditor`)
- Modify: `src/i18n.en.ts`, `src/i18n.pt.ts` (after `actions.permission`)
- Modify: `src/style.css` (next to `.action-badge`, ~1503)

**Interfaces:**
- Consumes: TS `Profile`, `validRules` (Task 2), `capabilitiesOf(...).readOnlyProfile` (Task 1).
- Produces: `ModelPickerOptions.accepts?: (provider: ProviderId) => boolean`; keys `actions.providerRule`, `actions.ruleFixed`, `actions.ruleDifferent`, `actions.candidates`, `actions.candidatesHint`, `actions.addCandidate`, `actions.moveUp`, `actions.access`, `actions.accessDefault`, `actions.accessReadOnly`, `actions.readOnlyHint`, `actions.readOnlyMcp`, `actions.readOnlyUnsupported`, `actions.differentCaption`, `actions.readOnly`.

- [ ] **Step 1: Add the copy**

`src/i18n.pt.ts` after `"actions.permission": …,`:

```ts
  "actions.providerRule": "Escolha do provider",
  "actions.ruleFixed": "Sempre este provider e modelo",
  "actions.ruleDifferent": "Um provider diferente do que construiu o workspace",
  "actions.candidates": "Providers em ordem de preferência",
  "actions.candidatesHint": "A tarefa usa o primeiro provider instalado e conectado que não construiu o workspace. Sem nenhum, usa o primeiro disponível e indica que a revisão é da mesma família.",
  "actions.addCandidate": "Adicionar provider",
  "actions.moveUp": "Subir",
  "actions.access": "Acesso",
  "actions.accessDefault": "Pode alterar arquivos, conforme as permissões abaixo",
  "actions.accessReadOnly": "Somente leitura, garantido pelo provider",
  "actions.readOnlyHint": "Tarefas somente leitura não editam arquivos, não executam comandos que escrevem e não usam servidores MCP. Leituras e inspeções rodam sem pedir permissão.",
  "actions.readOnlyMcp": "Tarefas somente leitura rodam sem servidores MCP.",
  "actions.readOnlyUnsupported": "{provider} não consegue garantir acesso somente leitura.",
  "actions.differentCaption": "Diferente de quem construiu",
  "actions.readOnly": "Somente leitura",
```

`src/i18n.en.ts` at the same position:

```ts
  "actions.providerRule": "Provider choice",
  "actions.ruleFixed": "Always this provider and model",
  "actions.ruleDifferent": "A provider other than the one that built the workspace",
  "actions.candidates": "Providers in order of preference",
  "actions.candidatesHint": "The task uses the first installed and signed-in provider that did not build the workspace. Without one, it uses the first available provider and marks the review as same family.",
  "actions.addCandidate": "Add provider",
  "actions.moveUp": "Move up",
  "actions.access": "Access",
  "actions.accessDefault": "Can modify files, following the permissions below",
  "actions.accessReadOnly": "Read-only, enforced by the provider",
  "actions.readOnlyHint": "Read-only tasks cannot edit files, run commands that write or use MCP servers. Reads and inspections run without asking.",
  "actions.readOnlyMcp": "Read-only tasks run without MCP servers.",
  "actions.readOnlyUnsupported": "{provider} cannot enforce read-only access.",
  "actions.differentCaption": "Different from the builder",
  "actions.readOnly": "Read-only",
```

- [ ] **Step 2: Let the model picker filter providers**

`src/model-picker.ts`: add `/** Hide providers the caller cannot use, such as those without a capability. */ accepts?: (provider: ProviderId) => boolean;` to `ModelPickerOptions`, and change `providers` to
`const providers = () => installed().filter(p => (!options.only || p.id === options.only) && (!options.accepts || options.accepts(p.id)));`

- [ ] **Step 3: Lockable MCP selection**

In `action-settings.ts` `selection()`, create `const note = h("p", "ui-hint"); note.hidden = true;`, append it after `list`, and return:

```ts
  return {
    root,
    get: () => inherit.control.checked ? null : choices.filter(c => c.checked.checked).map(c => c.id),
    /// Read-only profiles cannot use the axis; keep the selection visible but inert and say why.
    lock(locked: boolean, reason: string) {
      inherit.control.disabled = locked;
      list.disabled = locked || inherit.control.checked;
      note.hidden = !locked; note.textContent = reason;
    },
  };
```

- [ ] **Step 4: Rewrite the profile editor**

Imports: `import { capabilitiesOf, descriptor, installed, modelLabelOf, onCatalogChange } from "./agents";` and `import type { Choice, ProviderId } from "./types";`. Replace the body of `profileEditor`'s `sheet(...)` callback with:

```ts
    const name = input(profile.name); name.required = true;
    const prompt = input(profile.prompt, true); prompt.required = true;
    const choice = { ...profile.choice };
    const rows: Choice[] = profile.candidates.map(c => ({ ...c }));
    const rule = select(profile.provider_rule, [["fixed", t("actions.ruleFixed")], ["different_from_builder", t("actions.ruleDifferent")]]);
    const access = select(profile.access, [["default", t("actions.accessDefault")], ["read_only", t("actions.accessReadOnly")]]);
    const readOnly = () => access.value === "read_only";
    const capable = (agent: ProviderId) => capabilitiesOf(agent).readOnlyProfile;
    const accepts = (agent: ProviderId) => !readOnly() || capable(agent);
    const model = ui.button("", () => openModelPicker(model, {
      current: choice, accepts,
      select: selected => {
        const effort = fitsEffort(selected.model, choice.effort, selected.agent);
        const adjusted = effort !== choice.effort;
        Object.assign(choice, selected, { effort });
        model.title = adjusted ? t("models.effortAdjusted") : "";
        render();
      },
    }));
    const effort = ui.button("", () => openEffortPicker(effort, choice, choice.effort, value => {
      choice.effort = value;
      model.title = "";
      render();
    }));
    model.className = effort.className = "outline md pick";
    const effortField = field("actions.effort", effort);
    const fixed = h("div", "ui-columns");
    fixed.append(field("actions.model", model), effortField);
    const unsupported = h("p", "ui-hint");
    const list = h("div", "action-candidates");
    const add = ui.menuButton(t("actions.addCandidate"), () => installed()
      .filter(provider => !rows.some(row => row.agent === provider.id) && accepts(provider.id))
      .map(provider => ({ label: provider.label, run: () => { rows.push({ agent: provider.id, model: "", effort: "" }); render(); } })));
    const different = h("div", "action-rule");
    different.append(h("b", "action-tool-title", t("actions.candidates")), list, add, h("p", "ui-hint", t("actions.candidatesHint")));
    const candidate = (row: Choice, index: number) => {
      const root = h("div", "action-candidate");
      const label = descriptor(row.agent).label;
      const picker = ui.button(`${index + 1}. ${label} · ${modelLabel(row.model, row.agent)}`, () => openModelPicker(picker, {
        current: row, only: row.agent,
        select: selected => { Object.assign(row, selected, { effort: fitsEffort(selected.model, row.effort, selected.agent) }); render(); },
      }));
      picker.className = "outline md pick";
      picker.setAttribute("aria-label", `${t("actions.model")} · ${label}`);
      root.append(picker);
      const step = effortStep(row.model, row.effort, row.agent);
      if (step) {
        const bars = ui.button(step.label, () => openEffortPicker(bars, row, row.effort, value => { row.effort = value; render(); }));
        bars.className = "outline md pick";
        bars.setAttribute("aria-label", `${t("actions.effort")} · ${label}`);
        root.append(bars);
      }
      root.append(more(label, [
        { label: t("actions.moveUp"), disabled: index === 0, run: () => { rows.splice(index - 1, 0, ...rows.splice(index, 1)); render(); } },
        { label: t("actions.remove"), disabled: rows.length === 1, run: () => { rows.splice(index, 1); render(); } },
      ], `candidate-${index}`));
      const notes = [
        ...(descriptor(row.agent).installed ? [] : [t("actions.uninstalled")]),
        ...(readOnly() && !capable(row.agent) ? [t("actions.readOnlyUnsupported", { provider: label })] : []),
      ];
      if (notes.length) root.append(h("span", "action-caption", notes.join(" · ")));
      return root;
    };
    const servers = selection("actions.mcp", profile.mcp, mcp.list().map(s => s.id));
    const packages = selection("actions.plugins", profile.plugins, plugins.list().map(p => p.id));
    const skills = input(profile.skills.join(", "));
    const permission = select(profile.permission, [["ask", t("actions.ask")], ["auto", t("actions.auto")]]);
    const permissionField = field("actions.permission", permission.control);
    const readOnlyHint = h("p", "ui-hint", t("actions.readOnlyHint"));
    const render = () => {
      const differentRule = rule.value === "different_from_builder";
      fixed.hidden = differentRule;
      different.hidden = !differentRule;
      model.textContent = `${descriptor(choice.agent).label} · ${modelLabel(choice.model, choice.agent)}`;
      const step = effortStep(choice.model, choice.effort, choice.agent);
      effortField.hidden = !step;
      effort.textContent = step?.label ?? "";
      list.replaceChildren(...rows.map(candidate));
      permissionField.hidden = readOnly();
      readOnlyHint.hidden = !readOnly();
      servers.lock(readOnly(), t("actions.readOnlyMcp"));
      const blocked = (differentRule ? rows.map(row => row.agent) : [choice.agent]).filter(agent => readOnly() && !capable(agent));
      unsupported.textContent = blocked.map(agent => t("actions.readOnlyUnsupported", { provider: descriptor(agent).label })).join(" ");
      unsupported.hidden = differentRule || !blocked.length;
    };
    rule.onchange = () => {
      if (rule.value === "different_from_builder" && !rows.length) rows.push({ ...choice });
      if (rule.value === "fixed" && rows.length) Object.assign(choice, rows[0]);
      render();
    };
    access.onchange = render;
    render();
    const forgetCatalog = onCatalogChange(render);
    body.closest("dialog")!.addEventListener("close", forgetCatalog, { once: true });
    const watching = checkbox("actions.watch", !!profile.watch);
```

keep the existing watch controls (`watchBody`, `interval`, `limit`, `comments`, `ci`) unchanged, then:

```ts
    const tools = ui.disclosure(t("actions.tools"));
    tools.toggleAttribute("open", !!profile.mcp?.length || !!profile.plugins?.length || !!profile.skills.length);
    tools.append(servers.root, packages.root, field("actions.skills", skills), h("p", "ui-hint", t("actions.skillsHint")));
    body.append(field("actions.name", name), field("actions.instructions", prompt), field("actions.providerRule", rule.control),
      fixed, unsupported, different, field("actions.access", access.control), readOnlyHint, tools, permissionField,
      watching.label, watchBody, h("p", "ui-hint", t("actions.snapshotHint")));
    return () => {
      const differentRule = rule.value === "different_from_builder";
      const next = structuredClone(actions.catalog());
      const result: actions.Profile = { ...profile, name: name.value.trim(), prompt: prompt.value.trim(),
        choice: differentRule ? { ...rows[0] } : { ...choice },
        provider_rule: rule.value as actions.ProviderRule,
        candidates: differentRule ? rows.map(row => ({ ...row })) : [],
        access: access.value as actions.Access,
        mcp: readOnly() ? null : servers.get(), plugins: packages.get(), skills: skills.value.split(",").map(s => s.trim()).filter(Boolean),
        permission: permission.value as actions.Profile["permission"],
        watch: watching.control.checked ? { interval_seconds: Number(interval.value), max_turns: Number(limit.value), comments: comments.control.checked, ci: ci.control.checked } : null,
      };
      if (!actions.validRules(result, capable)) throw new Error(unsupported.textContent || t("err.actions.invalid"));
      if (project) { (next.overrides[project] ??= {})[result.id] = result; }
      else { next.profiles = [...next.profiles.filter(p => p.id !== result.id), result]; }
      return next;
    };
```

(The old `renderChoice` is replaced by `render`; remove the old `models` columns and the old `body.append`.)

- [ ] **Step 5: Describe the rule on profile cards**

In `settingsRows`, replace the card description argument with:

```ts
    const differentRule = profile.provider_rule === "different_from_builder";
    const providers = differentRule
      ? profile.candidates.map(c => descriptor(c.agent).label).join(" → ")
      : `${descriptor(profile.choice.agent).label} · ${model}`;
```

pass `providers` to `card(...)`, and after the `watchEnabled` caption:

```ts
    if (differentRule) item.text.append(h("span", "action-caption", t("actions.differentCaption")));
    if (profile.access === "read_only") item.text.append(h("span", "action-caption", t("actions.readOnly")));
```

- [ ] **Step 6: Style the candidate list**

`src/style.css`, after `.action-badge`:

```css
.action-rule, .action-candidates { display: grid; gap: 6px; }
.action-candidate { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; }
.action-rule > .ui-button { justify-self: start; }
```

- [ ] **Step 7: Verify**

Run: `npm run typecheck && npm run architecture:check && npx vitest run && npx playwright test e2e/actions.spec.ts e2e/settings.spec.ts`
Expected: PASS.

Then check by hand in the browser mock (`npm run dev`, Settings › Actions): the Code review card reads "Codex → Claude · Different from the builder · Read-only"; the editor shows two candidate rows, moving Claude up reorders them, removing leaves one row with Remove disabled; switching Access to "Can modify files…" shows Permissions and unlocks MCP; with Read-only, adding Antigravity is not offered and a fixed Antigravity choice shows "Antigravity cannot enforce read-only access." and saving refuses with that message.

- [ ] **Step 8: Commit**

```bash
git add src/model-picker.ts src/action-settings.ts src/i18n.en.ts src/i18n.pt.ts src/style.css
git commit -m "feat(actions): choose reviewer providers and read-only access in agent profiles" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: Task footer shows the reviewing provider and its guarantees

**Files:**
- Modify: `src/actions.ts` (`taskBadges`), `src/actions.test.ts`
- Modify: `src/components/chat/composer.ts:16-24, 46`
- Modify: `src/components/chat/chat.css:272-280`
- Modify: `src/chat.ts:14, 965-1081`
- Modify: `src/components/stories.ts:151-165`, `src/components/catalog.json:268-284`
- Modify: `src/i18n.en.ts`, `src/i18n.pt.ts`

**Interfaces:**
- Consumes: `TaskRun.same_family` (Task 3), `Profile.access` (Task 2), `actions.readOnly` key (Task 6).
- Produces: `taskBadges(run: TaskRun | null | undefined): { label: Key; title: Key }[]`; composer returns `badges: HTMLElement` (class `taskbadges`); keys `actions.readOnlyTitle`, `actions.sameFamily`, `actions.sameFamilyTitle`.

- [ ] **Step 1: Write the failing test**

`src/actions.test.ts` (import `taskBadges`, `type TaskRun`):

```ts
describe("task badges", () => {
  const run = (access: "default" | "read_only", same_family?: boolean) =>
    ({ profile: { access }, same_family } as unknown as TaskRun);
  it("explains enforced read-only access and a same-family fallback", () => {
    expect(taskBadges(null)).toEqual([]);
    expect(taskBadges(run("default"))).toEqual([]);
    expect(taskBadges(run("read_only", true)).map(b => b.label)).toEqual(["actions.readOnly", "actions.sameFamily"]);
    expect(taskBadges(run("default", true)).map(b => b.title)).toEqual(["actions.sameFamilyTitle"]);
  });
});
```

Run: `npx vitest run src/actions.test.ts` → FAIL.

- [ ] **Step 2: Implement `taskBadges` and the copy**

`src/actions.ts` (`import type { Key } from "./i18n";`):

```ts
/// How a task runs, beyond its model: read-only access enforced by the adapter and a same-family fallback.
export function taskBadges(run: TaskRun | null | undefined): { label: Key; title: Key }[] {
  if (!run) return [];
  return [
    ...(run.profile.access === "read_only" ? [{ label: "actions.readOnly", title: "actions.readOnlyTitle" } as const] : []),
    ...(run.same_family ? [{ label: "actions.sameFamily", title: "actions.sameFamilyTitle" } as const] : []),
  ];
}
```

`src/i18n.pt.ts` after `"actions.readOnly"`:

```ts
  "actions.readOnlyTitle": "Esta tarefa não pode alterar arquivos; o provider garante isso.",
  "actions.sameFamily": "Mesma família",
  "actions.sameFamilyTitle": "Nenhum outro provider instalado e conectado estava disponível, então esta revisão usa a família que construiu o workspace.",
```

`src/i18n.en.ts`:

```ts
  "actions.readOnlyTitle": "This task cannot modify files; the provider enforces it.",
  "actions.sameFamily": "Same family",
  "actions.sameFamilyTitle": "No other installed and signed-in provider was available, so this review uses the family that built the workspace.",
```

Run the test → PASS.

- [ ] **Step 3: Composer slot and styles**

`composer.ts`: after the `watch` button, `const badges = h("span", "taskbadges"); badges.hidden = true;`, change `meta.append(withModel, watch, hint);` to `meta.append(withModel, watch, badges, hint);` and add `badges` to the returned object.

`chat.css`, after `.composer .with.ro button`:

```css
.composer .taskbadges { display: inline-flex; flex: none; gap: 4px; }
```

- [ ] **Step 4: Paint the label and badges**

`chat.ts`: `import { capabilitiesOf, descriptor, onCatalogChange } from "./agents";` and `import { badge } from "./ui";`. In `paintWith` replace `const label = modelLabel(info.model, info.agent);` with:

```ts
    // A task freezes its provider, which can differ from the workspace's, so name it beside the model.
    const label = info.task ? `${descriptor(info.agent).label} · ${modelLabel(info.model, info.agent)}` : modelLabel(info.model, info.agent);
```

In `paintComposer`, right after the `if (run) { … }` block:

```ts
    const badges = this.box.querySelector<HTMLElement>(".taskbadges")!;
    const items = actions.taskBadges(run);
    badges.hidden = !items.length;
    badges.replaceChildren(...items.map(item => {
      const element = badge(t(item.label)); element.title = t(item.title);
      return element;
    }));
```

- [ ] **Step 5: Gallery state**

`catalog.json` composer `states`: add `"task"`. `stories.ts` composer story, before `return wrap(view.root);`:

```ts
    if (state === "task") {
      view.withModel.hidden = false;
      view.model.textContent = "Codex · Default";
      view.badges.hidden = false;
      view.badges.append(badge(t("actions.readOnly")), badge(t("actions.sameFamily")));
    }
```

importing `badge` from `./primitives` in `stories.ts`.

- [ ] **Step 6: Verify**

Run: `npm run typecheck && npm run architecture:check && npx vitest run && npx playwright test e2e/actions.spec.ts e2e/design-system.spec.ts e2e/composer.spec.ts`
Expected: PASS. In the browser mock (`npm run dev`), run `/review` in the "Hello" workspace, whose tabs run Claude: the new tab's footer reads "Codex · …" with a "Read-only" badge. Then remove Codex's account in the devtools console with `localStorage.setItem("mock:accounts", JSON.stringify({ accounts: [{ id: "claude", provider: "claude", email: "personal@example.com", plan: "max", connected: true, revision: 0 }], active: { claude: "claude" }, login: null })); location.reload();`, close the first review tab and run `/review` again: the footer reads "Claude · …" with "Read-only" and "Same family", and each badge's tooltip explains it. Restore with `localStorage.removeItem("mock:accounts")`. Finally open `/design-system.html?component=composer&state=task`.

- [ ] **Step 7: Commit**

```bash
git add src/actions.ts src/actions.test.ts src/components/chat/composer.ts src/components/chat/chat.css src/chat.ts src/components/stories.ts src/components/catalog.json src/i18n.en.ts src/i18n.pt.ts
git commit -m "feat(review): show the reviewing provider and read-only access on the task" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: Decisions, contracts, matrix and full verification

**Files:**
- Create: `docs/decisions/0064-cross-family-review.md`
- Modify: `docs/decisions/0009-reusable-actions.md`, `docs/decisions/0010-default-code-review.md`, `docs/decisions/README.md`
- Modify: `docs/contracts/actions.md`, `docs/contracts/agent-runtime.md`, `docs/contracts/persistence.md:194-198`
- Modify: `docs/quality/provider-matrix.md` (rows 75–77 and known limitations)
- Modify: `docs/README.md` (ADR list and implementation records), `README.md:70-71`

- [ ] **Step 1: Write ADR 0064**

```md
# ADR 0064 — Cross-family, read-only default review

Date: 2026-09-28
Status: Accepted

## Context

The bundled Code review profile from ⟦ADR 0010 | 0010-default-code-review.md⟧
pinned Claude and relied on its prompt and on approvals to stay read-only. When
the workspace ran Claude, the reviewer shared the builder's family: judges
prefer their own generations, and two instances of one model tend to make
correlated mistakes. A fresh session removes the builder's transcript, not the
family's blind spots. ⟦ADR 0009 | 0009-reusable-actions.md⟧ treats profile
instructions as guidance, so "do not modify files" was a request.

## Options considered

1. Keep a fixed provider and tighten the prompt: the family and the guarantee
   stay the same.
2. Pick the reviewer in the presentation and pass it to `action_start`: the
   installation and account checks would live where they can be stale.
3. A provider rule resolved by the backend at task start, an access level each
   adapter materializes, and a capability that tells the presentation which
   providers can enforce it.

## Decision

Adopt the third option. `provider_rule` is `fixed` or `different_from_builder`;
the latter carries ordered candidates, each with its own model and effort.
Builders are the providers of the workspace's ordinary tabs. At start, the
backend takes the first installed and signed-in candidate outside the builders,
then the first usable candidate, then the first candidate; it never refuses the
review. The frozen task records the picked candidate and whether it shares a
builder's family, and the tab shows both.

`access: read_only` is enforced by the adapter, not by the prompt. Claude runs
in restricted mode with `dontAsk`, read tools only, the worktree added for
`CLAUDE.md` and an inline empty MCP configuration; settings files cannot widen
it and bypass is refused. Codex runs in the `read-only` sandbox with
`approvalPolicy: never`. Antigravity cannot enforce it and advertises
`readOnlyProfile: false`. Read-only tasks run without MCP servers, whose tools
act outside both envelopes.

The bundled profile becomes `different_from_builder` with Codex then Claude at
provider defaults and read-only access. A catalog upgrades a profile identical
to the previous seed exactly once, recorded by `defaults_revision`; customized,
overridden and removed profiles stay as they are.

## Consequences

A review usually comes from another family and uses that provider's quota; the
status bar keeps showing each provider's account. Without another usable
provider, the review still runs and says it shares the builder's family. A
read-only reviewer cannot run tests that write caches or build outputs; checks
belong to a deterministic step outside the agent. Claude's restricted mode loads
the plugins and skills selected in Prometeu and the CLI's built-in plugins, not
plugins enabled in CLI settings or project skills, and needs `--restricted`
(verified with 2.1.283, present since at least 2.1.260); an older CLI fails the
spawn visibly. Codex does not report sandbox-denied commands as items; the
reviewer's text reports them. Git configuration left in the repository, such as
`core.fsmonitor`, still runs for any Git read, including Prometeu's own.

An older app ignores the new fields and runs the first candidate as a fixed
profile; if it saves the catalog, the fields are lost and the profile stays
fixed.

## Evidence

- ⟦Selection | ../../src-tauri/src/actions/reviewer.rs⟧ and
  ⟦validation, freezing and the seed upgrade | ../../src-tauri/src/actions.rs⟧.
- ⟦Claude arguments and recording | ../../src-tauri/src/claude.rs⟧,
  ⟦Codex parameters and recording | ../../src-tauri/src/codex.rs⟧ and the
  ⟦Antigravity refusal | ../../src-tauri/src/antigravity.rs⟧.
- ⟦Capability descriptor | ../../src-tauri/src/agents.rs⟧.
- ⟦Mirror, seed parity and badges | ../../src/actions.test.ts⟧.
- ⟦Contract | ../contracts/actions.md⟧ and
  ⟦provider matrix | ../quality/provider-matrix.md⟧.
```

- [ ] **Step 2: Amend ADRs 0009 and 0010 and the indexes**

ADR 0009, Decision: replace "Profiles have a prompt, model/provider/effort, MCP, plugins, skills, permissions and optional tracking." with "Profiles have a prompt, model/provider/effort or a provider rule, an access level, MCP, plugins, skills, permissions and optional tracking (⟦ADR 0064 | 0064-cross-family-review.md⟧)." and append to "…they are not deterministic gates of a workflow engine." the sentence "Read-only access is the exception: the adapter enforces it."

ADR 0010, Decision: append to "It does not publish a PR, modify files or follow CI by default." → "…by default. Since ⟦ADR 0064 | 0064-cross-family-review.md⟧, the adapter enforces its read-only access and it runs on a provider other than the builder's when one is available." and replace "The additive `defaults_initialized` field records the initialization." with "The additive `defaults_initialized` field records the initialization; `defaults_revision` records later seed upgrades, which replace only a profile identical to an earlier seed."

`docs/decisions/README.md` table: `| ⟦0064 | 0064-cross-family-review.md⟧ | Cross-family, read-only default review |`.

`docs/README.md`: after the ADR 0063 line add `- ⟦ADR 0064 | decisions/0064-cross-family-review.md⟧: the default review runs read-only, on another provider family when one is available.`.

- [ ] **Step 3: Update the contracts**

`docs/contracts/actions.md`:
- "Registry and entries": list `defaults_revision` with the other fields; after "The profile reports bugs and risks without modifying files or publishing a PR." add "It runs read-only on a provider other than the builder's when one is available. A catalog whose bundled profile is still identical to an earlier seed receives the current seed once; `defaults_revision` records it, so later edits stay."
- "Profile and execution": change the first sentence to include "`provider_rule: fixed | different_from_builder`, ordered `candidates` (provider/model/effort each), `access: default | read_only`". Replace the paragraph starting "Permissions are materialized by the adapter" with two subsections:

```md
### Provider rule

`fixed` runs `choice`. `different_from_builder` has one to three candidates with
distinct providers, and `choice` mirrors the first so an older app runs it as a
fixed profile. Builders are the providers of the workspace's ordinary tabs,
each tab's own choice or the workspace default it inherits; without ordinary
tabs, the workspace default counts. At start the backend takes the first
candidate that is installed, signed in and not a builder, then the first usable
candidate, then the first candidate, whose spawn reports why it cannot run.
Usable means the CLI is on the adopted PATH and the active account's last known
identity is signed in; Antigravity needs its version check and an attached
account. The provider is picked before tool resolution. `Tab.task.profile`
freezes the picked candidate as a fixed choice and `Tab.task.same_family`
records that no usable candidate outside the builders existed.

### Access level

Under `default`, `permission` applies: Claude uses its normal approval mode
under `ask`; Codex uses `approvalPolicy: untrusted`; `auto` keeps the existing
bypass. These options do not constitute worktree isolation.

`read_only` requires every provider the profile can start with to advertise
`readOnlyProfile`, and an MCP selection that is `null` or empty; the frozen task
has no MCP servers. Adapters materialize it:

- Claude: `--restricted --permission-mode dontAsk --tools
  Read,Grep,Glob,Bash,Skill,Task --add-dir <worktree>` with
  `CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD=1` and an inline empty strict
  MCP configuration. Settings files are ignored, write tools are absent in the
  session and its subagents, commands the CLI does not classify as read-only
  are denied without a prompt, and bypass is refused. Plugins and skills
  selected in Prometeu still load; plugins enabled only in CLI settings and
  project skills do not.
- Codex: `sandbox: "read-only"` and `approvalPolicy: "never"` on start and
  resume, with an empty `mcp_servers` table.
- Antigravity: refused before the spawn.

Read-only access limits what injected repository content can do to the review
text. It is not an operating-system sandbox for Claude's shell, and Git
configuration left in the repository still runs for any Git read.
```

- "IPC and compatibility": in `action_start`, "resolves the profile" → "resolves the profile and its provider"; add "Profiles and tasks saved before the provider rule load as `fixed`, `default` and `same_family: false`."

`docs/contracts/agent-runtime.md`: add `access: "default" | "read_only";` to `SessionLaunch` and the bullet "- `read_only` is materialized by adapters that advertise `readOnlyProfile` and refused by the others before spawning; it excludes plan mode and MCP servers;". In the Antigravity section, after "Nonempty hub selections fail at the adapter boundary." add "Read-only launches fail there too."

`docs/contracts/persistence.md`: extend "recorded in `actions.defaults_initialized`" with "; later seed revisions, recorded in `actions.defaults_revision`, replace only a profile identical to an earlier seed".

- [ ] **Step 4: Update the provider matrix and README**

Replace the "Code review included and editable" row:

```md
| Code review included and editable | second candidate; read-only through restricted `dontAsk` mode | first candidate; read-only through the `read-only` sandbox | may be a candidate of a default-access profile; cannot run read-only | `actions.rs`, `actions/reviewer.rs`, `actions.test.ts`; `e2e/actions.spec.ts` for profile editing |
```

Change the "per-task profile" row's Claude and Codex cells to "instructions, permissions and access through flags" / "instructions, permissions and access through JSON-RPC". Add after it:

```md
| reviewer from a family other than the builder's | candidate chosen by the shared backend rule | same rule | same rule when access is default | `actions/reviewer.rs` pick/builders tests, `actions.test.ts` mirror |
| read-only task profile | restricted `dontAsk`, read tools, worktree added for `CLAUDE.md`, empty strict MCP; recording from 2.1.283 | `read-only` sandbox, `approvalPolicy: never`, empty MCP table; recording from 0.154.0 | Unavailable (`readOnlyProfile: false`) | `claude.rs` and `codex.rs` launch and recording tests, `antigravity.rs` refusal |
```

Known limitations: add

```md
- read-only Claude tasks load plugins and skills selected in Prometeu and the
  CLI's built-in plugins, not plugins enabled in CLI settings or project skills,
  and need a CLI with `--restricted`;
- Codex 0.154.0 does not report sandbox-denied commands and patches as items in
  read-only tasks; the agent's text reports them;
```

`README.md`: replace "Reusable actions include an editable code review profile." with "Reusable actions include an editable code review profile that runs read-only, on another provider family than the one that built the workspace when one is installed and signed in."

- [ ] **Step 5: Full verification**

Run: `npm run docs:check && npm run check`
Expected: PASS end to end (docs, architecture, formatting, build, contracts, mobile build, release/web/Rust/E2E tests, clippy). If a check cannot run, record exactly which and why.

- [ ] **Step 6: Commit**

```bash
git add docs/decisions/0064-cross-family-review.md docs/decisions/0009-reusable-actions.md docs/decisions/0010-default-code-review.md docs/decisions/README.md docs/contracts/actions.md docs/contracts/agent-runtime.md docs/contracts/persistence.md docs/quality/provider-matrix.md docs/README.md README.md
git commit -m "docs(review): record the cross-family read-only review decision" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```
