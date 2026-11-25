---
name: writebook
description: |
  Read and edit Writebook books with the writebook CLI. Sign in to Writebook
  instances, list books and their contents, read pages as Markdown, and
  search. To edit a book, clone it, change its Markdown files, and push the
  changes back with git-style commands. Use for ANY task that reads or
  changes a Writebook book.
triggers:
  - writebook
  - /writebook
  - writebook instance
  - writebook profile
  - writebook book list
  - writebook book contents
  - writebook clone
  - writebook pull
  - writebook push
  - writebook status
  - writebook diff
  - writebook read
  - writebook search
  - edit a writebook book
  - add a page to a book
  - reorder book pages
---

# Writebook CLI

Writebook is a self-hosted web app by 37signals for writing and publishing books.
Each company or person can run one or multiple own copies of it, called an
instance. The writebook CLI can work with several instances.

You can use the CLI in two ways:

- Read books directly from an instance with the online commands. Nothing is
  saved to disk.
- Clone a book to a folder of Markdown files, edit the files, and push the
  changes back to the instance with the local commands. This works like git.

## Authentication

### Instances

The CLI keeps a list of instances. Each instance has a URL and a short name
called an alias. One instance is the default. The CLI uses the default
instance when you don't pass `--instance`.

```sh
writebook instance list --json          # all instances, which one is the default, and who is signed in
writebook instance add <url>            # sign in to an instance and add it to the list
writebook instance set-default <alias>
writebook instance remove <alias>       # sign out and remove the instance from the list
```

Before you ask the user for a Writebook URL, run `writebook instance list --json`.
The instance they mean is probably already in the list.

To sign in, run `writebook instance add <url>`. If the instance is already in
the list, the command signs in to it again. The first instance you add becomes
the default. `--alias <name>` sets the alias. Without it, the alias is the
instance's host name, for example `books.example.com`.

`--instance` accepts an alias or a URL. A URL also works for an instance that
isn't in the list.

### Profiles

A profile is a separate sign-in to the same instance, for example a personal
account and an editor account. Each instance starts with one profile called
`default`. To give the first profile a different name, pass `--profile <name>`
to `instance add`.

```sh
writebook profile add <name> [--instance <alias>]   # sign in as another user
writebook profile list [--instance <alias>]
writebook profile set-default <name> [--instance <alias>]
writebook profile remove <name> [--instance <alias>]
```

Every command uses the instance's default profile. To use a different
profile, pass `--profile <name>`.

### Signing in

Signing in uses the browser. The CLI prints a link. The user opens the link on
any device, signs in to Writebook, and approves the request. Writebook then
shows an approval code. The user types the code into the CLI, or waits until
the CLI sees the approval. The CLI stores the sign-in in the system keyring
and renews it when it expires.

When you run the CLI as an agent, the user can't type into the CLI, so signing
in takes these steps:

1. Run `writebook instance add <url>`. It prints a link.
2. Give the link to the user and ask them to approve it. Ask them for the
   approval code that Writebook shows after they approve.
3. Run the same command again, with `--code <code>` added.

If a command needs a sign-in and there is none, the command doesn't wait for
the user. It fails, and the error message says which command to run.

If you only need to read books, add `--read-only` in step 1. The sign-in can
read books but can't change them, so `push` fails. The approval page tells
the user that the device will only be able to read.

### Tokens

Instead of signing in, you can set two environment variables:

- `WRITEBOOK_TOKEN`: an API token.
- `WRITEBOOK_TOKEN_HOST`: the URL of the instance that the token is for. The
  CLI sends the token only to this instance.

The CLI uses the token as it is. It doesn't save the token or renew it.

## Online commands

These commands read from the instance and print the result:

```sh
writebook book list [--instance <alias>]       # the books on an instance, with their ids
writebook book contents <book-id-or-url>       # the pages, sections, and pictures in a book, in order
writebook read <book-or-page-url>              # a book or a page as Markdown
writebook search "<query>" [--book <book-id-or-url>]
```

- `book contents` accepts a book URL, or a book id together with
  `--instance`. Without `--instance`, it looks for the book id on the default
  instance.
- `book contents` needs a sign-in. The other commands can read published books
  without one.
- `read` accepts a book URL or a page URL, including edit links and links
  that end in `.md`. It prints the Markdown with front matter that holds the
  title and the URL.
- `search` with `--book` searches that book. `--book` accepts a book URL, or
  a book id together with `--instance`. Without `--book`, `search` searches
  every book on the instance. It returns at most 50 results.

### Output formats

`book list`, `book contents`, `search`, and `instance list` print a table by
default. The table is for people. When you need to use the output, pass
`--json`. `--markdown` prints a Markdown list.

`--json` prints this structure:

```json
{
  "ok": true,
  "data": "...",
  "breadcrumbs": [
    { "action": "...", "command": "...", "description": "..." }
  ]
}
```

`data` holds the result. `breadcrumbs` lists commands that you can run next.

`--jq <expression>` runs a jq filter on the JSON and prints the result. You
don't need jq installed. If the result is a string, it is printed without
quotes, the same as `jq -r`.

```sh
writebook book list --jq '.data[] | "\(.id) \(.title)"'
writebook book contents <book-url> --jq '.data.leaves[] | select(.type == "page") | .url'
writebook read <page-url> --jq '.data.markdown'
```

## Local commands

To edit a book, clone it, edit its files, and push the changes:

```sh
writebook clone <book-url> [folder]   # download the book into a new folder
writebook status                      # show what changed locally and on the instance since the last sync
writebook diff [--remote]             # show your changes, or the instance's changes with --remote
writebook pull                        # download changes from the instance and merge them into your files
writebook push                        # upload your changes
writebook remote add|list|remove      # manage the instances this folder syncs with
```

Run these commands inside the cloned folder. They sign in as the instance's
default profile. To use a different profile, pass `--profile <name>`.

### What a cloned book looks like

```
index.md            the book's settings and its table of contents
getting-started.md  one file for each page, with only the page's Markdown
assets/             the cover, pictures, and images used in pages
.writebook/         files the CLI uses to sync. Don't edit them.
```

### index.md

The front matter holds the book's settings. Below it is the table of contents,
with one line for each page, section, and picture. The order of the lines is
the order of the book.

```markdown
---
title: The Field Guide
subtitle:
author: Stanko
theme: green
cover: assets/cover.jpg
---

# Part One {theme=blue}               ← a section with the blue theme
# [Part Two](part-two.md)             ← a section with text, stored in part-two.md
[Getting Started](getting-started.md) ← a page. The link text is the page title.
![A sunset](assets/sunset.jpg)        ← a picture. The alt text is the caption.
```

- To change the book's title, author, or theme, edit the front matter.
- To set or replace the cover, put the image in `assets/` and write its path
  after `cover:`. To remove the cover, leave `cover:` empty.
- To change a section's theme, write `{theme=<name>}` at the end of its line.
  The themes are black, blue, green, magenta, orange, violet, and white.
- To rename a page, edit its link text. Don't rename the file.
- To move a page, move its line.
- To add a page, create a `.md` file, add a line for it in `index.md`, and
  push.
- To delete a page, remove its line and its file, and push. The instance
  moves the page to the trash.
- A page file contains only the page's text. Don't put the title or front
  matter in it.

### Images

To add an image to a page, put the file in `assets/` and write
`![description](assets/name.png)` in the page. When you push, the CLI uploads
the image and renames the file to the name the instance gives it. The CLI
also updates the reference in the page.

To replace a picture, overwrite its file in `assets/`.

### Syncing

- Run `writebook status` before and after you edit.
- If someone changed the book on the instance since your last pull, `push`
  stops. Run `writebook pull`, then push again.
- `pull` merges the instance's changes into your files. If you and someone
  else changed the same lines, `pull` adds `<<<<<<<` and `>>>>>>>` conflict
  markers to the file. Edit the file to resolve the conflict and remove the
  markers. `pull` and `push` refuse to run while a file has conflict markers.
- If someone edits the book on the web while you push, the push fails with a
  409 error. Run `writebook pull`, then push again.
