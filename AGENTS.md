# writebook-cli - Agent instructions

writebook-cli is the command line, TUI and MCP server for Writebook. Read `README.md` for what
it does.

## Development

`bin/setup` installs everything needed to work on this project.

Every development task goes through `bin/dev`, so every agent does it the same way:

```sh
bin/dev code lint     # Check the formatting, clippy and the style rules
bin/dev code review   # Lint, then review the branch against doc/review.md
bin/dev test run      # Run the tests
```

The tool is strict on purpose and has few options. Add a command to it, in
`crates/writebook-dev`, rather than a one-off script.

Read the code-style skill (`.agents/skills/code-style/SKILL.md`) before writing or changing
Rust. `doc/review.md` holds the rules a pull request is reviewed against.

Before you open a pull request, run `bin/dev code lint` and `bin/dev test run`. Once it's open,
run `bin/dev code review` and fix what it finds.
