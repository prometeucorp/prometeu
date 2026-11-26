---
type: Operations
title: "Technical writing in ASD-STE100"
description: "How to write and review Prometeu technical documentation in ASD-STE100 Simplified Technical English, Issue 9."
status: stable
---

# Technical writing in ASD-STE100

All technical documentation in this repository uses ASD-STE100 Simplified
Technical English (STE), Issue 9. This rule applies to each new document and to
each change to a document. [ADR 0107](../decisions/0107-simplified-technical-english.md)
records the decision.

The specification is the official edition from ASD:
<https://www.asd-ste100.org/assets/files/ASD-STE100_ISSUE9.pdf>.
Use the specification for the full rules and the dictionary. This page does not
copy the specification. It gives the scope, the project terms and the review
procedure.

## Scope

The rule applies to these files:

- The root documents: `README.md`, `CONTRIBUTING.md`, `AGENTS.md`, `CLAUDE.md`,
  `ARCHITECTURE.md` and `SECURITY.md`.
- All Markdown files in `docs/`.
- The `README.md` files in `packages/`, `src/` and `src-tauri/`. These files
  include the descriptions of the test fixtures.
- The GitHub issue templates and the pull request template in `.github/`.
- The changelog header in `cliff.toml`. git-cliff copies this header into
  `CHANGELOG.md`.

The rule does not apply to these files:

- Text from third parties and licenses, for example `LICENSE` and
  `packages/terminal-state/THIRD_PARTY_NOTICES.md`.
- The release entries in `CHANGELOG.md`. git-cliff makes them from commit
  messages, and they are a historical record.
- Prompt files for agent commands in `.claude/commands/`. These files are
  instructions for a tool, not documentation.
- Interface text and i18n catalogs. Interface text has its own rules in the
  [design system](../architecture/design-system.md).
- Comments in source code and commit messages. Clear English is sufficient
  there.

## Text that does not change

STE applies to the prose. Do not change these items when you edit a document:

- Commands, code blocks, inline code, file paths and identifiers.
- Names of APIs, parameters, fields, events, IPC commands and configuration
  keys.
- Link targets and heading anchors that other documents use. If you change a
  heading, update all the links to its anchor in the same change.
- Text that you copy accurately from the interface, a protocol or an external
  tool. Quoted text counts as one word.
- Names of products, companies and persons.
- YAML keys and the `type` and `status` values in the front matter. The `title`
  and `description` values are prose. Write them in STE.

## How to apply the rules

The specification has nine sections of writing rules. This summary tells you how
each section applies to this repository. The rule numbers refer to Issue 9.

### Words (section 1)

- Use the approved words of the dictionary. Use each word only as its approved
  part of speech and with its approved meaning.
- Use the technical nouns and technical verbs in this page. Software terms are
  technical nouns (category 19). Computer processes are technical verbs
  (category 2 of the technical verbs).
- Use one term for one item. Use the term that the code and the contracts use.
- Do not use a technical noun as a verb. For example, "diff" is a technical
  noun. Write "compare the files", not "diff the files".
- Use American English spelling.

### Multi-word nouns (section 2)

- Do not write a noun cluster of more than three words. Use a preposition to
  divide it. For example, write "the state of the board in the desktop", not
  "desktop board state store".
- A name from the code, for example `AgentCapabilities`, is one word.

### Verbs (section 3)

- Use only these verb forms: the infinitive, the imperative, the simple present
  and the simple past. You can also use the simple future with "will" and the
  past participle as an adjective.
- Use an "-ing" form only in a technical noun, for example "logging" or
  "pairing". Do not use the present perfect, the past perfect or the
  progressive.
- Use the active voice. Tell which part of the system does the action. In a
  description, use the passive voice only when you do not know which part does
  the action.
- Do not use "should", "may", "might", "would" or "shall". Use "must" for a
  requirement and "can" for a possibility.

### Sentences (section 4)

- Write short and clear sentences. Do not remove articles or other words to
  make a sentence shorter. Do not use contractions.
- Use a vertical list when a sentence has a series of items or conditions.
- Use "the", "a", "an", "this" or "these" before a noun when it is applicable.

### Procedures (section 5)

- A step in a procedure has a maximum of 20 words.
- Write one instruction in each step. Use the imperative.
- When the reader must know a condition first, start the step with the
  condition. For example: "If the check shows an error, read the log."
- Use a note only to give information. Do not put an instruction in a note.

### Descriptions (section 6)

- A sentence in a description has a maximum of 25 words.
- Give the information in a logical sequence. Start with the general information.
- Each paragraph has one topic and a maximum of six sentences.

### Safety instructions (section 7)

- Start a warning or a caution with a clear command. Then tell the reader the
  risk. Use a warning for a risk to persons and a caution for a risk to data,
  credentials or equipment.

### Punctuation and word count (section 8)

- Do not use the semicolon. Write two sentences.
- A number, a code identifier, a path, a quoted text and a heading count as one
  word. Text in parentheses counts as one word.
- In a vertical list, a colon has the same function as a period. It is the
  end of the sentence before the list.

### Writing practices (section 9)

- If a word-for-word replacement does not give a clear sentence, write a new
  sentence.
- Do not use phrasal verbs, for example "set up" or "find out". Use "prepare"
  or "find".
- Use "that" after a verb that introduces a clause, for example "make sure that".
- Do not use Latin abbreviations, for example "e.g.", "i.e." or "etc." Write
  "for example" or "that is", or give the full list.

## Project technical nouns

These terms are technical nouns. Use each term only for the item in its
definition. If a necessary term is not in this list, use the term from the code
or the contract. Add the term to this list in the same change.

| Term | Item |
| --- | --- |
| `agent` | An external coding tool that does work in a workspace, for example Claude Code or Codex. |
| `provider` | The company or product that supplies an agent and its models. |
| `account` | The login of a person for a provider or for Prometeu Cloud. |
| `workspace` | A unit of work with one git worktree and its tabs. |
| `sub-workspace` | A workspace that a delegation makes in a parent workspace. |
| `worktree` | A git worktree that isolates the files of one workspace. |
| `repository` | A git repository. |
| `project` | A repository that the board shows, with its workspaces. |
| `board` | The data model of the projects, workspaces and tabs. |
| `desk` | The home screen with one view for each conversation that operates. |
| `tab` | One conversation, terminal or tool in a workspace. |
| `conversation` | The messages between a person and an agent in a tab. |
| `session` | The transcript of a conversation. The agent process can stop and start again on the same session. |
| `transcript` | The stored record of the messages and events of a session. |
| `turn` | One prompt and the agent work that it causes. |
| `prompt` | The text that a person or an action sends to an agent. |
| `attachment` | A file or an image that goes with a prompt. |
| `event` | A message that tells about a change. It is not the general meaning "occurrence". |
| `request` | A message that tells a process or a server to do an operation or to send an answer. |
| `response` | The message that answers a request. |
| `payload` | The data in a message. |
| `process` | A program that the operating system operates. It is not the general meaning "procedure". |
| `state` | The stored values of a part of the system at a time. |
| `capability` | A function of an agent that the application can use. It agrees with `AgentCapabilities`. |
| `feature` | A function of the product that a person can see or use. |
| `action` | A saved prompt or command that a person can start again from the interface. |
| `hook` | A command that an agent or git starts at a specified time. |
| `profile` | A set of settings for an agent account or a tool. |
| `setting` | A value that a person can change in Settings. |
| `relay` | The Cloudflare Worker that sends encrypted messages between devices. |
| `Prometeu Cloud` | The optional service for accounts, organizations, shared sessions and supervision. |
| `organization` | A group of Prometeu Cloud accounts that share a catalog. |
| `companion device` | A device that follows a shared session. The session stays on the Mac of the owner. |
| `owner` | The person who has the Mac that operates a shared session. |
| `remote machine` | A computer that Prometeu connects to through SSH. |
| `host` | The `prometeu-host` daemon on a remote machine. |
| `contract` | A document in `docs/contracts/` that specifies a format or a guarantee. |
| `ADR` | An architecture decision record in `docs/decisions/`. |
| `adapter` | Code that changes an external protocol into the internal format. |
| `port` | An interface through which the core uses an external service. |
| `core` | The portable rules in `src-tauri/crates/core/`. |
| `fixture` | Recorded data that a test uses. |
| `check` | A command that examines the repository and shows if a rule is not obeyed. |
| `test` | A check that operates code and compares the result with the correct result. |
| `build` | The result of a compilation, or the procedure that makes it. |
| `release` | A published version of the application. |
| `failure` | A condition where an operation does not give the expected result. |
| `key` | A cryptographic key, or the name of a value in a map. |
| `token` | A unit of text for a model, or a secret that gives access. Tell which one. |
| `cache` | A copy of data that the application keeps to read it again quickly. |
| `log` | A file or a stream of records about operations. |
| `option` | A value that a person or a command can select. |
| `field` | A named value in a structure or a form. |
| `list` | A sequence of items. |
| `file`, `folder`, `path` | Items of the file system. |
| `code` | Source code. |
| `application`, `app` | The Prometeu desktop application, or a different program when the text tells which one. |

## Project technical verbs

These verbs are technical verbs for computer processes. Use them only with the
meaning in the table. If an approved verb gives the same meaning, use the
approved verb.

| Verb | Meaning |
| --- | --- |
| `run` | To start and operate a command, a script, a test or a check. For other meanings, use "operate". |
| `build` | To make an application or a package from the code. |
| `commit`, `push`, `pull`, `merge`, `rebase`, `fetch`, `clone`, `stage` | The git operations with the same names. |
| `publish` | To make a release, a package or a shared item available to other persons. |
| `deploy` | To install a service, for example the relay, on its server. |
| `install`, `uninstall`, `update`, `upgrade` | To add, remove or change a version of software. |
| `download`, `upload` | To copy data from or to a server. |
| `load`, `save`, `store`, `delete`, `copy`, `paste` | The usual computer operations on data. |
| `encrypt`, `decrypt`, `sign` | The cryptographic operations. |
| `authenticate`, `log in`, `sign in`, `log out` | To prove the identity of a person or a device. |
| `parse`, `serialize`, `render`, `validate`, `filter`, `sort` | The usual operations on data and on the interface. |
| `click`, `type`, `enter`, `scroll`, `drag` | The operations of a person in the interface. |
| `enable`, `disable` | To make a function available or not available. |
| `call` | To start a function, a command or an API. For other meanings, use "tell". |
| `return` | To give a value back to the caller of a function or a command. |
| `resume` | To start an agent process again on a session that it started before. |
| `stream` | To send data in parts while the operation continues. |
| `debug` | To find and repair errors in code. |

## Frequent replacements

The specification gives the full list of words that are not approved. These
replacements occur frequently in this repository:

| Do not write | Write |
| --- | --- |
| should, shall | must |
| may, might | can, or "it is possible that" |
| ensure | make sure |
| allow | let |
| need, require | is necessary |
| provide | give, supply |
| support (as a verb) | can + verb, or "operates with" |
| handle | use, do, process |
| exist | be, there is |
| remain | stay |
| retain, preserve, persist | keep |
| create | make |
| existing | current, or "that is there" |
| every | all, each |
| both | the two |
| already | before, at this time |
| still | continue to, stay |
| never | not ... at any time |
| once (conjunction) | when, after |
| instead | an alternative, or write a new sentence |
| however | but |
| therefore | thus, as a result |
| via | through |
| over (a protocol) | through |
| i.e., e.g., etc. | that is, for example, or the full list |

## Review

Do this review for each pull request that adds or changes technical
documentation. The author does the review first. Then the reviewer does it
again.

1. Make sure that the changed text uses approved words, project technical nouns
   or project technical verbs.
2. Make sure that each word has its approved part of speech and meaning.
3. Make sure that each procedure step has a maximum of 20 words.
4. Make sure that each descriptive sentence has a maximum of 25 words.
5. Make sure that the text uses the active voice and the permitted verb forms.
6. Make sure that the text has no semicolons, contractions or Latin
   abbreviations.
7. Make sure that the technical meaning did not change.
8. Make sure that commands, examples, identifiers and links did not change.
9. Run `npm run docs:check`.

A word that is not in the dictionary or in this page is an error. Replace the
word, or add a project technical noun or verb to this page with a definition.
