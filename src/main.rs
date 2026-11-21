mod api;
mod checkout;
mod clone;
mod completions;
mod creds;
mod diff_cmd;
mod fsutil;
mod index;
mod merge;
mod plan;
mod pull;
mod push;
mod read;
mod remote;
mod setup;
mod skill;
mod status;
mod sync;
mod web;

use anyhow::Result;
use std::path::PathBuf;
use usage::{Cli, Subcommands};

/// Sync Writebook books to disk like git repositories
#[derive(Cli)]
#[usage(bin = "writebook", version, unknown_flags = "error", completion)]
struct Cli {
    /// Token to use instead of signing in (for agents and scripts)
    #[usage(long, global, env = "WRITEBOOK_TOKEN")]
    token: Option<String>,
    #[usage(subcommand)]
    command: Command,
}

#[derive(Subcommands)]
enum Command {
    /// Clone a book from a Writebook site into a new directory
    Clone {
        /// Book URL, like https://books.example.com/4/some-book
        url: String,
        /// Directory to clone into (defaults to the book's slug)
        directory: Option<PathBuf>,
    },
    /// Manage the remotes this checkout syncs with
    Remote {
        #[usage(subcommand)]
        command: RemoteCommand,
    },
    /// Fetch remote changes and merge them into your files
    Pull,
    /// Upload your local changes to the remote
    Push,
    /// Show what changed locally and remotely since the last sync
    Status,
    /// Show local changes since the last sync
    Diff {
        /// Diff the remote's changes instead of yours
        #[usage(long)]
        remote: bool,
    },
    /// List the books on a Writebook site
    Books {
        /// Site URL, like https://books.example.com
        url: String,
        /// Print JSON instead of Markdown
        #[usage(long)]
        json: bool,
    },
    /// Print a book or one page as Markdown, without cloning
    Read {
        /// Book or page URL, like https://books.example.com/4/some-book/12/some-page
        url: String,
        /// Print JSON instead of Markdown
        #[usage(long)]
        json: bool,
    },
    /// Search a book, or every book on a site
    Search {
        /// Book URL, or a site URL to search all its books
        url: String,
        /// What to look for
        query: String,
        /// Print JSON instead of Markdown
        #[usage(long)]
        json: bool,
    },
    /// Sign in to a Writebook site. Without a terminal, run it once to get the
    /// link to approve, then again with --code to finish
    Login {
        /// Site URL, like https://books.example.com
        url: String,
        /// The approval code Writebook showed after you approved
        #[usage(long)]
        code: Option<String>,
    },
    /// Set up writebook: agent skill, shell completions, and a quick tour
    Setup,
    /// Print or install the agent skill for coding agents
    Skill {
        #[usage(subcommand)]
        command: SkillCommand,
    },
    /// Print or install shell completions
    ShellCompletion {
        #[usage(subcommand)]
        command: ShellCompletionCommand,
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
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    if let Err(error) = run(Cli::parse()) {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    if let Some(token) = &cli.token {
        creds::use_token(token);
    }

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
        Command::Books { url, json } => read::books(&url, json),
        Command::Read { url, json } => read::read(&url, json),
        Command::Search { url, query, json } => read::search(&url, &query, json),
        Command::Login { url, code } => creds::login_command(&web::parse_target(&url)?.host, code.as_deref()),
        Command::Setup => setup::run(),
        Command::Skill { command } => match command {
            SkillCommand::Print => skill::print(),
            SkillCommand::Install => skill::install(),
        },
        Command::ShellCompletion { command } => match command {
            ShellCompletionCommand::Print { shell } => completions::print(&shell),
            ShellCompletionCommand::Install { shell } => completions::install(&shell),
        },
    }
}
