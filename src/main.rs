mod api;
mod checkout;
mod clone;
mod completions;
mod creds;
mod diff_cmd;
mod fsutil;
mod index;
mod instance;
mod inventory;
mod mcp;
mod merge;
mod output;
mod plan;
mod pull;
mod profile;
mod push;
mod read;
mod remote;
#[cfg(test)]
mod scratch;
mod setup;
mod skill;
mod status;
mod sync;
mod tui;
mod upgrade;
mod web;

use anyhow::Result;
use std::path::PathBuf;
use usage::{Cli, Subcommands};

use output::Format;

/// Sync Writebook books to disk like git repositories
#[derive(Cli)]
#[usage(bin = "writebook", version, unknown_flags = "error", completion, subcommand_help_heading = "Online commands", disable_help_subcommand)]
struct Cli {
    /// Profile to sign in as, instead of the instance's default one
    #[usage(long, global, env = "WRITEBOOK_PROFILE")]
    profile: Option<String>,
    #[usage(subcommand)]
    command: Command,
}

#[derive(Subcommands)]
enum Command {
    /// Manage the Writebook instances you use
    #[usage(help_heading = "Auth commands")]
    Instance {
        #[usage(subcommand)]
        command: InstanceCommand,
    },
    /// Manage an instance's profiles, separate sign-ins to the same site
    #[usage(help_heading = "Auth commands")]
    Profile {
        #[usage(subcommand)]
        command: ProfileCommand,
    },
    /// Clone a book from a Writebook site into a new directory
    #[usage(help_heading = "Local commands")]
    Clone {
        /// Book URL, like https://books.example.com/4/some-book
        url: String,
        /// Directory to clone into (defaults to the book's slug)
        directory: Option<PathBuf>,
    },
    /// Manage the remotes this checkout syncs with
    #[usage(help_heading = "Local commands")]
    Remote {
        #[usage(subcommand)]
        command: RemoteCommand,
    },
    /// Fetch remote changes and merge them into your files
    #[usage(help_heading = "Local commands")]
    Pull,
    /// Upload your local changes to the remote
    #[usage(help_heading = "Local commands")]
    Push,
    /// Show what changed locally and remotely since the last sync
    #[usage(help_heading = "Local commands")]
    Status,
    /// Show local changes since the last sync
    #[usage(help_heading = "Local commands")]
    Diff {
        /// Diff the remote's changes instead of yours
        #[usage(long)]
        remote: bool,
    },
    /// Work with the books on an instance
    Book {
        #[usage(subcommand)]
        command: BookCommand,
    },
    /// Print a book or one page as Markdown, without cloning
    Read {
        /// Book or page URL, like https://books.example.com/4/some-book/12/some-page
        url: String,
        /// Print JSON instead of Markdown
        #[usage(long)]
        json: bool,
        /// Filter the JSON with a jq expression, like '.data.markdown'
        #[usage(long)]
        jq: Option<String>,
    },
    /// Search a book, or every book on a site
    Search {
        /// What to look for
        query: String,
        /// Book id or URL to search in (without it, every book on the instance)
        #[usage(long)]
        book: Option<String>,
        /// Instance alias or URL (defaults to the default instance)
        #[usage(long)]
        instance: Option<String>,
        /// Print JSON
        #[usage(long, conflicts = "markdown")]
        json: bool,
        /// Print Markdown
        #[usage(long)]
        markdown: bool,
        /// Filter the JSON with a jq expression, like '.data[].url'
        #[usage(long, conflicts = "markdown")]
        jq: Option<String>,
    },
    /// Set up writebook: agent skill, shell completions, and a quick tour
    #[usage(help_heading = "Setup commands")]
    Setup,
    /// Print or install shell completions
    #[usage(help_heading = "Setup commands")]
    ShellCompletion {
        #[usage(subcommand)]
        command: ShellCompletionCommand,
    },
    /// Upgrade writebook to the latest release
    #[usage(help_heading = "Setup commands")]
    Upgrade,
    /// Let agents read and edit books on every instance you added, through
    /// MCP: `claude mcp add writebook -- writebook mcp`. With --serve it gives
    /// anyone's agent access to one instance over HTTP at /mcp, using their own
    /// login: every request must include a Writebook token as a Bearer header.
    /// It serves plain HTTP, so put it behind https unless it only listens on
    /// this machine
    #[usage(help_heading = "Agent commands")]
    Mcp {
        /// With --serve, the instance alias or URL to serve (defaults to the default instance)
        #[usage(long)]
        instance: Option<String>,
        /// Serve only the tools that read
        #[usage(long)]
        read_only: bool,
        /// Serve over HTTP at this address, like 127.0.0.1:8080, instead of stdio
        #[usage(long)]
        serve: Option<String>,
    },
    /// Browse your instances' books in the terminal, and arrange their pages
    Tui,
    /// Print or install the agent skill for coding agents
    #[usage(help_heading = "Agent commands")]
    Skill {
        #[usage(subcommand)]
        command: SkillCommand,
    },
}

#[derive(Subcommands)]
enum SkillCommand {
    /// Print the embedded SKILL.md
    Print,
    /// Install the skill into ~/.agents/skills (and link it for Claude Code)
    Install,
}

#[derive(Subcommands)]
enum InstanceCommand {
    /// Sign in to an instance and add it; the first one becomes the default.
    /// Without a terminal, run it once to get the link to approve, then again
    /// with --code to finish
    Add {
        /// Site URL, like https://books.example.com, or the alias of one you added
        url: String,
        /// Short name for it (defaults to its host name)
        #[usage(long)]
        alias: Option<String>,
        /// The approval code Writebook showed after you approved
        #[usage(long)]
        code: Option<String>,
        /// Sign in so this device can read books but not change them
        #[usage(long)]
        read_only: bool,
    },
    /// List instances and whether you're signed in to each
    List {
        /// Print JSON
        #[usage(long, conflicts = "markdown")]
        json: bool,
        /// Print Markdown
        #[usage(long)]
        markdown: bool,
        /// Filter the JSON with a jq expression, like '.data[].url'
        #[usage(long, conflicts = "markdown")]
        jq: Option<String>,
    },
    /// Sign out of an instance, all its profiles, and remove it
    Remove {
        /// Instance alias or URL
        instance: String,
    },
    /// Use this instance when --instance isn't given
    SetDefault {
        /// Instance alias or URL
        instance: String,
    },
}

#[derive(Subcommands)]
enum ProfileCommand {
    /// Sign in to an instance as another profile. Without a terminal, run it
    /// once to get the link to approve, then again with --code to finish
    Add {
        /// Profile name, like editor
        name: String,
        /// Instance alias or URL (defaults to the default instance)
        #[usage(long)]
        instance: Option<String>,
        /// The approval code Writebook showed after you approved
        #[usage(long)]
        code: Option<String>,
        /// Sign in so this device can read books but not change them
        #[usage(long)]
        read_only: bool,
    },
    /// List an instance's profiles; * marks the default
    List {
        /// Instance alias or URL (defaults to the default instance)
        #[usage(long)]
        instance: Option<String>,
    },
    /// Sign out of a profile and remove it
    Remove {
        /// Profile name
        name: String,
        /// Instance alias or URL (defaults to the default instance)
        #[usage(long)]
        instance: Option<String>,
    },
    /// Use this profile when --profile isn't given
    SetDefault {
        /// Profile name
        name: String,
        /// Instance alias or URL (defaults to the default instance)
        #[usage(long)]
        instance: Option<String>,
    },
}

#[derive(Subcommands)]
enum BookCommand {
    /// List a book's pages, sections and pictures, in order
    Contents {
        /// Book id on the instance, or a book URL like https://books.example.com/4/some-book
        book: String,
        /// Instance alias or URL, for a book id (defaults to the default instance)
        #[usage(long)]
        instance: Option<String>,
        /// Print JSON
        #[usage(long, conflicts = "markdown")]
        json: bool,
        /// Print Markdown
        #[usage(long)]
        markdown: bool,
        /// Filter the JSON with a jq expression, like '.data[].url'
        #[usage(long, conflicts = "markdown")]
        jq: Option<String>,
    },
    /// List the books on an instance
    List {
        /// Instance alias or URL (defaults to the default instance)
        #[usage(long)]
        instance: Option<String>,
        /// Print JSON
        #[usage(long, conflicts = "markdown")]
        json: bool,
        /// Print Markdown
        #[usage(long)]
        markdown: bool,
        /// Filter the JSON with a jq expression, like '.data[].url'
        #[usage(long, conflicts = "markdown")]
        jq: Option<String>,
    },
}

#[derive(Subcommands)]
enum RemoteCommand {
    /// Add a remote
    Add {
        /// Remote name, like origin
        name: String,
        /// Book URL on the other Writebook site
        url: String,
    },
    /// List remotes
    List,
    /// Remove a remote
    Remove {
        /// Remote name
        name: String,
    },
}

#[derive(Subcommands)]
enum ShellCompletionCommand {
    /// Print the completion script to stdout
    Print {
        #[usage(choices("bash", "zsh", "fish"), choices_strict = false)]
        shell: String,
    },
    /// Install the completion script for your shell
    Install {
        #[usage(choices("bash", "zsh", "fish"), choices_strict = false)]
        shell: String,
    },
}

fn main() {
    // Die quietly on a closed pipe (`writebook skill print | head`) instead
    // of panicking, the way every other Unix CLI does.
    #[cfg(unix)]
    sigpipe::reset();

    if let Err(error) = run(Cli::parse()) {
        eprintln!("error: {}", output::terminal_safe(&format!("{error:#}")));
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    if let Ok(token) = std::env::var("WRITEBOOK_TOKEN") {
        creds::use_token(&token, std::env::var("WRITEBOOK_TOKEN_HOST").ok().as_deref());
    }

    inventory::select_profile(cli.profile.as_deref());

    match cli.command {
        Command::Clone { url, directory } => clone::run(&url, directory.as_deref()),
        Command::Remote { command } => match command {
            RemoteCommand::Add { name, url } => remote::add(&name, &url),
            RemoteCommand::List => remote::list(),
            RemoteCommand::Remove { name } => remote::remove(&name),
        },
        Command::Pull => pull::run(),
        Command::Push => push::run(),
        Command::Status => status::run(),
        Command::Diff { remote } => diff_cmd::run(remote),
        Command::Instance { command } => match command {
            InstanceCommand::Add { url, alias, code, read_only } => {
                instance::add(&url, alias.as_deref(), cli.profile.as_deref(), code.as_deref(), creds::Access::from_flag(read_only))
            }
            InstanceCommand::List { json, markdown, jq } => instance::list(Format::from_flags(json, markdown, jq)?),
            InstanceCommand::Remove { instance } => instance::remove(&instance),
            InstanceCommand::SetDefault { instance } => instance::set_default(&instance),
        },
        Command::Profile { command } => match command {
            ProfileCommand::Add { name, instance, code, read_only } => {
                profile::add(&name, instance.as_deref(), code.as_deref(), creds::Access::from_flag(read_only))
            }
            ProfileCommand::List { instance } => profile::list(instance.as_deref()),
            ProfileCommand::Remove { name, instance } => profile::remove(&name, instance.as_deref()),
            ProfileCommand::SetDefault { name, instance } => profile::set_default(&name, instance.as_deref()),
        },
        Command::Book { command } => match command {
            BookCommand::List { instance, json, markdown, jq } => {
                read::books(&inventory::host(instance.as_deref())?, Format::from_flags(json, markdown, jq)?)
            }
            BookCommand::Contents { book, instance, json, markdown, jq } => {
                read::contents(&book, instance.as_deref(), Format::from_flags(json, markdown, jq)?)
            }
        },
        Command::Read { url, json, jq } => read::read(&url, Format::from_flags(json, false, jq)?),
        Command::Search { query, book, instance, json, markdown, jq } => {
            read::search(&query, book.as_deref(), instance.as_deref(), Format::from_flags(json, markdown, jq)?)
        }
        Command::Mcp { instance, read_only, serve } => mcp::run(instance.as_deref(), read_only, serve.as_deref()),
        Command::Tui => tui::run(),
        Command::Setup => setup::run(),
        Command::Skill { command } => match command {
            SkillCommand::Print => skill::print(),
            SkillCommand::Install => skill::install(),
        },
        Command::ShellCompletion { command } => match command {
            ShellCompletionCommand::Print { shell } => completions::print(&shell),
            ShellCompletionCommand::Install { shell } => completions::install(&shell),
        },
        Command::Upgrade => upgrade::run(),
    }
}
