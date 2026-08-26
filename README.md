# writebook

THIS IS AN EXPERIMENT!

I want to enable agents to do sweeping changes in Writebook, and the best interface for that is, in my opinion, git.
So why don't we make Writebook talk git? Or something that resembles git, at least.

This CLI adds a few git-like commands that allow you to clone a book from any instance,
edit it as markdown locally, and then push the changes back up. Merge conflicts are supported!
So, in case mutliple edits occur, you'll get a conflict to resolve instead of clobbering each others work.

This is just an API wrapper with a 3-way-merger bolted-on.

---

A git-inspired CLI for [Writebook](https://once.com/writebook). It clones a book
to disk as plain Markdown files, so people — and agents — can edit with normal
tools and sync with the commands they already know:

```sh
writebook clone https://books.example.com/4/field-notes
cd field-notes

# edit files, move lines in index.md to reorder pages...

writebook status
writebook diff
writebook pull
writebook push
```

First run? `writebook setup` installs the agent skill, wires up shell
completions, and prints the basics.

## Commands

| Command | What it does |
|---|---|
| `setup` | Install the agent skill + shell completions, print a quick tour |
| `skill print/install` | Print the embedded SKILL.md, or install it for coding agents |
| `clone <book-url> [dir]` | Sign in (browser device flow) and download the book |
| `remote add/list/remove` | Manage the remotes in `.writebook/config.json` |
| `status` | What changed locally and remotely since the last sync |
| `diff [--remote]` | Unified diff of your changes (or the remote's) |
| `pull` | Fetch remote changes and three-way merge them into your files |
| `push` | Upload your changes; refuses when you're behind the remote |
| `shell-completion print/install <shell>` | Shell completions |

## A checkout

```
field-notes/
  index.md           the book: metadata up top, table of contents below
  getting-started.md one file per page: just the markdown body
  part-one.md        a section's body text, when it has one
  assets/            cover, picture images, images embedded in pages
  .writebook/        remotes, sync state, base snapshot (gitignored)
```

`index.md` starts with front matter for the book's metadata, then one line per
leaf — and **line order is book order**, so reordering pages means moving lines:

```markdown
---
title: Field Notes
subtitle:
author: Stanko
theme: green
---

# Part One                            ← a section
# [Part Two](part-two.md)             ← a section with body text
[Getting Started](getting-started.md) ← a page; link text is the title
![A sunset](assets/sunset.jpg)        ← a picture; alt text is the caption
```

Adding a line (and its file) creates a leaf on push; deleting a line trashes
it. Page files hold only the body — titles live in `index.md`. Images embedded
in a page reference `assets/` by relative path, so local Markdown preview just
works; push uploads new images and pull mirrors remote ones.

## How syncing works

`.writebook/base/` keeps a snapshot of every file as of the last sync — the
common ancestor. `pull` runs a three-way merge (base, yours, remote) per file:
non-overlapping edits combine cleanly, overlapping ones get standard conflict
markers to resolve before the next sync. `push` sends each update with the
fingerprint it was based on, and the server answers 409 rather than clobber an
edit you haven't seen — pull, merge, push again.

Authentication is an OAuth device flow: the CLI prints a short code, you
approve it in a signed-in browser, tokens land in
`~/.config/writebook/credentials.json` and refresh themselves. No token ever
lives inside a checkout.

## Server requirements

The Writebook instance needs the CLI API (OAuth device flow, the book
manifest, and JSON leaf CRUD) — shipped in the companion Writebook changes.

## Building

```sh
cargo build --release   # binary at target/release/writebook
cargo test
```
