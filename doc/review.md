# Review

These are the rules we review a pull request against.

`bin/dev code review` runs the lint, then has two reviewers check the branch's diff against
`main`'s version of this document. When the branch has a pull request and its head is pushed, it
posts the verdict as the `review` status on the head commit.

## How to review

- Judge outcomes and evidence, not taste. A finding names what goes wrong: a leaked token, a
  wrong answer, a lost edit, a broken rule.
- A finding cites a rule from this document by its id. Don't invent rules.
- Ask for the smallest correct change. Don't ask for refactors, renames, new abstractions, or
  work the change wasn't meant to do.
- When unsure, don't block. Pass, and say what you're unsure about in the summary. Comments are
  the exception: a comment you're unsure about fails STYLE-3.

## Rules

### Security

writebook-cli signs in to self-hosted Writebook instances and keeps their tokens.

#### SEC-1: Tokens stay secret

A token, refresh token or device code never reaches a log line, an error message, a command's
arguments, the URL handed to the browser opener, or any host other than the instance it belongs
to. If it's sent over plain http a prominent warning is displayed.

#### SEC-2: Credentials live in owner-only places

Tokens are kept in the system keyring, never inside a checkout. Any file or folder that holds
credentials or sign-in state is created readable and writable by its owner only, on every
platform, from the moment it's created.

#### SEC-3: No unsafe code

Nothing adds `unsafe`, or loosens the `unsafe_code = "forbid"` lint. Reach for a crate or a
subprocess instead.

### Testing

#### TEST-1: No test loosened to pass

Don't weaken, skip, ignore or delete a test, case or assertion to make the change pass.

#### TEST-2: No test depends on time

A test checks what happened, never how long it took. No sleeping for something to happen, no
limits on elapsed time, no race against a timeout. A fake server's route may hold or delay its
answer to stand in for a slow server, while the test itself still waits on a state. A deadline is
allowed only as a hang guard of a minute or more, on a wait for a state, as `eventually`'s is.

#### TEST-3: Tests check behavior through the public surface

A test drives the CLI, the TUI, the MCP server or a module's public functions, and checks what
they do. It never tests a private function directly, and never stubs or mocks the app's own code.
Fake Writebook servers and other stand-ins for what's outside the app are fine.

#### TEST-4: A change in behavior is tested

A change that adds or changes what the CLI, TUI or MCP server does comes with a test that fails
without it. A bug fix comes with a test that fails on the bug.

### Documentation

#### DOC-1: README and SKILL.md match what the CLI does

A change that adds, removes or changes a command, a flag, an MCP tool or what one of them does
updates `README.md` and `skills/writebook/SKILL.md` to match.

#### DOC-2: Plain wording

Docs, help text, error messages and output say what they mean in plain words. No metaphors,
personified code or clever phrasing where a literal phrase exists. An error says what went wrong
and, when there's something to do about it, what to do.

### Style

#### STYLE-1: The code follows the code-style skill

The change follows `AGENTS.md` and the code-style skill (`.agents/skills/code-style/SKILL.md`). A
fail names the rule that was broken and what goes wrong because of it. Guidance that wasn't
followed but causes no problem is no fail.

#### STYLE-2: Errors are handled, not dropped

An error the program is designed to meet is handled. `unwrap`, `expect` and similar functions
are only for a state that is a bug, where crashing is the only right answer, and the `expect`
names the broken invariant. Using them as a shortcut around error handling fails.

#### STYLE-3: No comment without a reason the code can't carry

The default is no comment. One stays only when:

- something outside our control forces the code (a dependency, platform, protocol or Writebook
  itself), and the code would look wrong without saying so.
- it's a contract a caller can't read off the signature: what a function guarantees, refuses or
  bounds.
- it links an upstream issue, RFC or spec.
- it's legal or license text.

Everything else fails: narration, restated names, history, justifications for a workaround,
"never", "must", "for now". A shortened bad comment is still a bad comment. A surprise means the
code is unclear: rename, extract, add a type. A rule becomes a type, an assertion or a test.
`bin/dev code lint` refuses the comments a pattern can catch; this rule is for the rest.

## Outcome

Each reviewer answers pass or fail. A fail lists its findings, each citing a rule id, a
`path:line` and what to change. The review fails if either reviewer fails.

## Changing this document

The review always reads this document from `main`, so a change to it applies from the next pull
request on.
