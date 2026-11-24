# Writebook CLI

Read, clone and edit books on any Writebook instance.

## Installation

With [mise](https://mise.jdx.dev):

```sh
mise use -g github:basecamp/writebook-cli
```

On Omarchy:

```sh
omarchy-mise-install github:basecamp/writebook-cli writebook
```

To upgrade, run `writebook upgrade`.

## Usage

Sign in to your Writebook. The CLI prints a link to approve in the browser:

```sh
writebook instance add https://books.example.com
```

Find a book and read it:

```sh
writebook book list
writebook book contents 4
writebook read https://books.example.com/4/field-notes/12/getting-started
writebook search "roll back" --book 4
```

To edit a book, clone it into a folder of Markdown files:

```sh
writebook clone https://books.example.com/4/field-notes
cd field-notes
```

Each page is a Markdown file. `index.md` holds the book's settings and its
table of contents. The order of its lines is the order of the book:

```markdown
---
title: Field Notes
author: Stanko
theme: green
cover: assets/cover.jpg
---

# Part One {theme=blue}               ← a section
[Getting Started](getting-started.md) ← a page
![A sunset](assets/sunset.jpg)        ← a picture
```

Edit the files with any tool, then sync like with git:

```sh
writebook status
writebook pull
writebook push
```

To work with more than one Writebook, add each one under a short name and pick
it with `--instance`:

```sh
writebook instance add https://handbook.example.org --alias handbook
writebook book list --instance handbook
```

Run `writebook --help` for every command.

## Development

```sh
cargo build --release
cargo test
```

`tests/live.rs` runs against a real Writebook:

```sh
docker run -d --name writebook -p 3000:80 -e SECRET_KEY_BASE=$(openssl rand -hex 64) -e DISABLE_SSL=1 ghcr.io/basecamp/writebook:main
docker exec -i writebook bin/rails runner - < tests/support/seed.rb
WRITEBOOK_LIVE_URL=http://localhost:3000 cargo test --test live
```

To publish a release, see [RELEASING.md](RELEASING.md).
