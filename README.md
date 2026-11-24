# writebook

THIS IS AN EXPERIMENT!

I want to enable agents to do sweeping changes in Writebook, and the best interface for that is, in my opinion, git.
So why don't we make Writebook talk git? Or something that resembles git, at least.

This CLI adds a few git-like commands that allow you to clone a book from any instance,
edit it as markdown locally, and then push the changes back up. Merge conflicts are supported!
So, in case mutliple edits occur, you'll get a conflict to resolve instead of clobbering each others work.

This is just an API wrapper with a 3-way-merger bolted-on.

THIS DEPENDS ON: https://github.com/basecamp/writebook/pull/482

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

Only need to look something up? `book list`, `book contents`, `read` and
`search` print straight to stdout, no checkout needed. The lists print a table,
or Markdown with `--markdown`, or JSON with `--json`. JSON comes as
`{ok, data, breadcrumbs}`, with suggested next commands in `breadcrumbs`, and
`--jq` filters it with a built-in jq (strings print raw, like `jq -r`):

```sh
writebook instance add https://books.example.com --alias books
writebook book list --instance books
writebook read https://books.example.com/4/field-notes/12/getting-started
writebook search "roll back" --book https://books.example.com/4/field-notes
writebook book contents 4 --jq '.data.leaves[] | select(.type == "page") | .url'
```

First run? `writebook setup` installs the agent skill, wires up shell
completions, and prints the basics.

## Commands

| Command | What it does |
|---|---|
| `instance add <url> [--alias <name>] [--code <code>]` | Sign in to an instance and add it; without a terminal, in two steps (link, then code) |
| `instance list/remove/set-default` | List instances, sign out of one and remove it, pick the default |
| `profile add <name> [--code <code>]` | Sign in to an instance as another profile |
| `profile list/remove/set-default` | List profiles, sign out of one and remove it, pick the default |
| `book list [--instance <instance>]` | List the books on an instance |
| `book contents <book-id-or-url> [--instance <instance>]` | List a book's pages, sections and pictures, in order |
| `read <url>` | Print a book or one page as Markdown |
| `search <query> [--book <book-id-or-url>] [--instance <instance>]` | Search a book, or every book on an instance |
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
cover: assets/cover.jpg
---

# Part One {theme=blue}               ← a section with its theme
# [Part Two](part-two.md)             ← a section with body text
[Getting Started](getting-started.md) ← a page; link text is the title
![A sunset](assets/sunset.jpg)        ← a picture; alt text is the caption
```

Adding a line (and its file) creates a leaf on push; deleting a line trashes
it. `cover:` names the cover image in `assets/` (empty for none), and a
section's theme goes at the end of its line as `{theme=...}`. Page files hold only the body — titles live in `index.md`. Images embedded
in a page reference `assets/` by relative path, so local Markdown preview just
works; push uploads new images and pull mirrors remote ones.

## How syncing works

`.writebook/base/` keeps a snapshot of every file as of the last sync — the
common ancestor. `pull` runs a three-way merge (base, yours, remote) per file:
non-overlapping edits combine cleanly, overlapping ones get standard conflict
markers to resolve before the next sync. `push` sends each update with the
fingerprint it was based on, and the server answers 409 rather than clobber an
edit you haven't seen — pull, merge, push again.

Authentication is an OAuth device flow. The CLI prints a link, and you
approve it in a signed-in browser on any device: nothing has to open on the
machine the CLI runs on. Writebook then shows an approval code; type it into
the CLI to finish, or just wait for the CLI to notice the approval. Tokens land
in the system keyring (or `~/.config/writebook/credentials.json` when there is
none) and refresh themselves. No token ever
lives inside a checkout.

### Instances and profiles

Writebook is self-hosted, so the CLI keeps an inventory of the instances you
use in `~/.config/writebook/instances.toml`. The first one you add becomes the
default, used whenever you don't pass `--instance`. `--instance` takes an alias
or a URL; a URL works even when it isn't in the inventory.

Adding an instance signs in to it, and removing one signs out. Adding one
that's already there signs in again.

Each instance has one or more profiles: separate sign-ins to the same site.
The first is called `default`, unless you name it with `--profile` when adding
the instance. Pick another one with `--profile` on any command.

```sh
writebook instance add https://books.example.com --alias work
writebook profile add editor --instance work
writebook book list --profile editor        # work is the default instance
writebook profile set-default editor --instance work
```

### Signing in without a terminal

Where nothing can type into the CLI (an agent, a script), sign in in two
steps:

```sh
writebook instance add https://books.example.com                  # prints the link to approve
writebook instance add https://books.example.com --code WXYZ-2345 # the code Writebook showed
```

The link works for 10 minutes. Approval codes need a Writebook with
basecamp/writebook#516; older ones still sign in by waiting at a terminal.

### Tokens, for agents and scripts

`WRITEBOOK_TOKEN` replaces the stored sign-in for any command:
the token is sent as is, never refreshed and never stored. Name the site it
belongs to with `WRITEBOOK_TOKEN_HOST`, like
`https://books.example.com`: the token is only ever sent there, so following a
link to another site can't leak it, and never over plain http except to this
machine. When there's no token and no stored sign-in, the browser sign-in only starts if
someone is at a terminal to approve it. Run unattended (no terminal, or `CI`,
`CLAUDECODE` or `WRITEBOOK_NO_SIGN_IN` set), the CLI exits with an error right
away instead of waiting.

## Reading without a checkout

`read` takes any link to a book or page: the reading link, the editing link,
or either with `.md`, `?search=` or `#anchor` on the end.

- With a token, it reads through the CLI API, so private books work.
- Otherwise, or when the site has no CLI API, it reads Writebook's Markdown
  export (a page's link plus `.md`), which serves published books without
  signing in.

`book list` and `search` use Writebook's JSON book list and search
(`/books.json`, `/search.json` and `/books/:id/search.json`), which a
read-only token may call. Searching a whole site is one request and returns
the best 50 matches across its books. On a Writebook without them, they read
the library page and send each book's search form instead, and a read-only
token cannot send that form. Without a token they see published books only.

## Server requirements

Syncing (`clone`, `pull`, `push`) needs the CLI API (OAuth device flow, the
book manifest, and JSON leaf CRUD) — shipped in the companion Writebook
changes. `book list`, `read` and `search` work with any recent Writebook, for
published books.

## Building

```sh
cargo build --release   # binary at target/release/writebook
cargo test
```

`tests/live.rs` runs the read-only commands against a real Writebook. CI runs
it against the `ghcr.io/basecamp/writebook:main` image; locally:

```sh
docker run -d --name writebook -p 3000:80 -e SECRET_KEY_BASE=$(openssl rand -hex 64) -e DISABLE_SSL=1 ghcr.io/basecamp/writebook:main
docker exec -i writebook bin/rails runner - < tests/support/seed.rb
WRITEBOOK_LIVE_URL=http://localhost:3000 cargo test --test live
```

CI builds release binaries for Linux (x86_64, arm64) and macOS (arm64) on
every change. Pushing a `v*` tag publishes them as a GitHub release.
