---
type: Decision
title: "ADR 0107 — Technical documentation in Simplified Technical English"
description: "All technical documentation uses ASD-STE100 Simplified Technical English, Issue 9."
status: stable
---

# ADR 0107 — Technical documentation in Simplified Technical English

Date: 2026-11-26

## Context

The technical documentation of Prometeu is in English. Many contributors and
agents read it, and English is not the first language of all of them. The
documents had long sentences, many synonyms for one item and many verb forms.
These problems make a document difficult to read and difficult to translate.
Persons and agents also wrote new text in different styles.

## Options considered

1. Keep free English and a short list of style recommendations.
2. Use a general plain-language guide.
3. Use ASD-STE100 Simplified Technical English (STE), Issue 9, with a project
   list of technical nouns and technical verbs.

## Decision

Adopt option 3. All technical documentation in the repository uses STE, Issue 9.
The rule applies to each new document and to each change to a document. The
[technical writing guide](../operations/technical-writing.md) gives the scope,
the exclusions, the project technical nouns and verbs, and the review procedure.

The official specification is the reference:
<https://www.asd-ste100.org/assets/files/ASD-STE100_ISSUE9.pdf>. The repository
does not contain a copy of the specification text.

STE controls only the prose. Commands, code, identifiers, link targets and
quoted text do not change.

## Consequences

- A reader gets one term for one item and short sentences with a small set of
  verb forms.
- An author must use the dictionary of the specification and the project list
  of technical terms. This takes more time than free English.
- Some software terms are not in the STE dictionary. The project list gives them
  as technical nouns or technical verbs with a specified meaning. An author adds
  a new term to that list in the same change.
- The rule does not apply to third-party text, licenses, generated release
  entries, agent command prompts, interface text, code comments and commit
  messages.
- The repository has no automatic STE check. The author and the reviewer do the
  review in the guide.

## Evidence

- [Technical writing guide](../operations/technical-writing.md)
- [Agent guide](../../AGENTS.md)
- [Contribution guide](../../CONTRIBUTING.md)
- [Pull request template](../../.github/PULL_REQUEST_TEMPLATE.md)
