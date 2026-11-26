---
name: code-style
description: How Rust is written here: 37signals conventions applied to Rust. Use before writing or changing any Rust in this repository, before writing or keeping a comment, when naming a type, module or function, when splitting a file, when a review raises a style question, and when a clippy lint or `bin/dev code lint` check tempts you toward an `#[allow]` or a workaround.
---

# Code style

Agents copy what they see. One oddly shaped function is the next agent's reason to write
another. Write code worth copying, and leave what you touch cleaner than you found it. Where
nearby code strays from this skill, follow the skill. A small cleanup goes in the change; a big
one gets named in the PR.

A code change can make a document wrong. When it changes what the CLI, the TUI or the MCP server
does, update `README.md` and `skills/writebook/SKILL.md` in the same commit.

Clippy and the lints of `bin/dev code lint` hold the rest of the style, and each failure says what
to write instead. Never quiet a check to make a change fit.

## Reader load

Code is good when a reader understands it with little work: few layers between a question and
its answer, little state to hold in their head.

Code is an asset and a liability. Every line has to be read, compiled, tested and kept working,
so more code costs more tokens, slower builds and more places for bugs. Keep that in mind.
That doesn't mean writing dense code. Don't code golf. But don't empire build either.

- A new reader finds where a value comes from and what can change it in half a minute.
- Every layer hides a real decision. Collapse one-caller wrappers and pass-throughs.
- State stays small and local: a value over a mutation, a local over a field, a field over
  anything shared.
- State an invariant once, at the boundary that owns it.
- Subtract before you add. No layer, option, dependency or helper the change doesn't need.
- Boring and clear beats clever.

## Names

- Name a thing for what it is or does in the domain, never for how it's built, what it looks
  like, when it runs or which platform needs it. `sign_in`, not `on_enter`. The trigger lives in
  the wiring.
- Use Writebook's words: a book, its leaves (pages, sections and pictures), an instance, a
  profile, a sign-in.
- No cryptic short names, no acronyms a newcomer wouldn't know.
- If a name needs a comment, the name is wrong.

## Model the domain

- Put the domain in a data structure (an enum, a state type, a table) rather than spreading it
  over conditionals. The right structure deletes branches.
- A type per meaning, not a bare string or integer. Arguments whose order can't be mixed up.
- A struct's definition is its state. No `Option` that only means "not set yet": build it whole,
  or split it into two types for two phases.
- Parse what comes from outside at the boundary, Writebook's JSON and the user's input alike;
  trust your own types inside it.

## Conditionals

Prefer an `if`/`else` that produces a value over an early return. `?` isn't a guard clause.
Combinators are fine when they say something the branch doesn't: `map` over an `Option` you
hand on, `and_then` to chain lookups.

## Fail fast

An `Option` or `Result` is either a state the program is designed to be in, or a bug. Find out
which before you choose. A bug panics with an `expect` naming the broken invariant, so the panic
reads like a bug report. A state the program is designed to be in is handled, never dropped:
`unwrap`, `expect` and `unwrap_or_default` aren't shortcuts around error handling.

## Errors

The CLI uses `anyhow`, with `context` saying what was being done. An error says what went wrong
and, when there's something to do about it, what to do. A state a caller acts on, such as a
conflict on save, gets its own check instead of a string match. An error never carries a token.

## Boundaries

- Nothing blocks the TUI's drawing: a request runs on a thread and comes back as a message.
- Nothing blocks inside `async`. When something must, it goes behind one small function, not
  `spawn_blocking` sprinkled through callers.
- A token goes only to the instance it belongs to. Sign-in files are owner-only from the moment
  they're created.

## Layout

- A constant that is a document (HTML, JSON, a prompt) lives in its own file and comes in with
  `include_str!`.
- A `From` lives with the type it produces.
- Everything else in the order it's called, top to bottom.
- One subject per file, named for it. Five short subjects are five short files. Signs you
  skipped it: a file you'd describe with "and", an enum whose variants each deserve a file.
- Past 500 lines, consider a split, whoever adds to it. Split by subject: `foo.rs` becomes
  `foo/` and a file per subject. Move first, change after, so the move reads as a move.
- Struct fields in reading order: identity, what it shows, then flags.
- Don't add a crate for a few lines of `std`; don't hand-roll base64, hashing, JSON or TLS. Turn
  off default features you don't use.
- A dependency's idioms aren't ours. A crate's examples show off the crate; they aren't a model
  for code written here.

## Comments

The default is no comment, so when unsure, cut it. One stays only when:

- something outside our control forces the code (a dependency, platform, protocol or Writebook
  itself), and the code would look wrong without saying so.
- it's a contract a caller can't read off the signature: what a client sees, what a function
  guarantees, refuses or bounds. It goes in a doc comment on a `pub` item.
- it links an upstream issue, RFC or spec.
- it's legal or license text.

Everything else goes: narration, restated names, history, justifications for a workaround,
"never", "must", "for now". Never shorten a bad comment into a smaller one. A test's only doc
comment is a link to the issue it holds, and a force goes beside the line it forces.

The comment lint (`crates/writebook-dev/src/lint/comments.rs`) refuses the mechanical cases and
says what to write instead: banners, to-do markers and commented-out code, `//` lines in a test,
a doc comment on a test with no link, a doc comment on an item that isn't `pub`, and comments
telling history or making an excuse. The lint can't see narration or restated names; the
reviewer judges those (`doc/review.md`, STYLE-2).

Deleting a comment doesn't delete what it knew. A surprise means the code is unclear: rename,
extract, add a type. A rule ("only under this lock", "within this bound") becomes a type, an
assertion or a test. If you can't encode it in this change, say so in the PR.

## Tests

- A test reads as one story: set up, act, assert. Name it for the behaviour.
- Through the public surface: the CLI's commands, the TUI's keys and screen, the MCP server's
  tools, a module's public functions. Never a private function, and never a stub or mock of the
  app's own code.
- Against real things where they're ours, and a fake Writebook server where they aren't. Never a
  trait that exists only for a test.
- Several related assertions in one test. Exact values, exact order where it's deterministic.
- A helper only for substantial repetition, in that test module.
- Never on how long something took. Read a timeout back from what the code asked for, count the
  work done, and wait on a state (`eventually`). A deadline is fine only as a hang guard of a
  minute or more.
- Each test runs in its own process under nextest (`bin/dev test run`): set what you need
  yourself.
- A bug fix comes with a test that fails on the bug.
