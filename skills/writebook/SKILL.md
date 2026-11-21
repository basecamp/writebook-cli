---
name: writebook
description: |
  Read and edit Writebook books with the writebook CLI. Read a book or page
  as Markdown, list books and search without cloning; or clone a book, edit
  pages as plain files, reorder by moving lines in index.md, and sync with
  git-style pull/push. Use for ANY task that reads or changes a Writebook book.
triggers:
  - writebook
  - /writebook
  - writebook clone
  - writebook pull
  - writebook push
  - writebook status
  - writebook diff
  - writebook read
  - writebook search
  - read a handbook page
  - runbook in writebook
  - edit a writebook book
  - add a page to a book
  - reorder book pages
---

# Writebook CLI

Writebook books sync to disk like git repositories. Work with normal file
tools; sync with git-style commands.

## Commands

```sh
writebook clone <book-url> [dir]   # e.g. https://books.example.com/4/field-notes
writebook status                   # local + remote changes since last sync
writebook diff [--remote]          # unified diff of your (or the remote's) changes
writebook pull                     # fetch + three-way merge remote changes
writebook push                     # upload local changes (refuses when behind)
writebook remote add|list|remove   # manage remotes
```

`clone` signs in through a browser device flow: it prints a link, the user
approves it in a signed-in browser on any device, and either types the
approval code Writebook then shows, or waits for the CLI to notice.
Tokens are stored per host and refresh themselves. A token from `--token` or
`WRITEBOOK_TOKEN` replaces the browser sign-in.

## Just reading

To look something up, don't clone. These print to stdout:

```sh
writebook read <book-or-page-url>        # Markdown with title/url front matter
writebook read <url> --json              # {title, url, markdown, source}
writebook books <site-url> [--json]      # the site's books
writebook search <book-or-site-url> "query" [--json]   # matching pages, with links
```

`read` accepts reading links, editing links and `.md` links. Published books
need no sign-in. For private books, pass a token: `--token <token>` or
`WRITEBOOK_TOKEN`.

## Running unattended

Set `WRITEBOOK_TOKEN` (or pass `--token`). Without a token or a stored
sign-in, commands that need one fail right away when nobody is at a terminal;
they never wait for a browser approval.

To sign in with no terminal, use two steps: run `writebook login <site-url>`,
give the user the link it prints to approve, ask them for the approval code
Writebook shows them, then run `writebook login <site-url> --code <code>`.

## A checkout

```
index.md           the book: metadata + table of contents
getting-started.md one file per page: just the Markdown body
assets/            cover, picture images, images embedded in pages
.writebook/        sync metadata — never edit by hand
```

## index.md rules

Front matter carries the book's metadata; the lines below are the table of
contents, one line per leaf. **Line order is book order** — reorder pages by
moving lines, never with a command.

```markdown
---
title: The Field Guide
subtitle:
author: Stanko
theme: green
---

# Part One                            ← section
# [Part Two](part-two.md)             ← section with body text in a file
[Getting Started](getting-started.md) ← page; link text is the title
![A sunset](assets/sunset.jpg)        ← picture; alt text is the caption
```

- Rename the book or change its author/theme by editing the front matter.
- Retitle a page by editing its link text; the filename stays.
- Add a page: create the `.md` file, add its line where it belongs, push.
- Delete a page: remove its line (and file); push trashes it on the server.
- Page files contain only the body — no title, no front matter.

## Images

Reference images in a page as `![alt](assets/name.png)` with the file placed
in `assets/`. Push uploads new images and renames the local file to the
server's slug — expect the reference to be rewritten. Replace a picture by
overwriting its file in `assets/`.

## Sync etiquette

- Run `writebook status` before and after editing.
- `push` refuses when the remote moved: run `writebook pull`, which three-way
  merges. Non-overlapping edits merge cleanly; overlapping edits leave
  standard `<<<<<<<`/`>>>>>>>` conflict markers. Resolve the markers, then
  push — pull and push both refuse while markers remain.
- A concurrent web edit during push answers 409; the fix is the same: pull,
  then push.

## Limits

Section themes and brand-new covers are edited on the web, not locally.
