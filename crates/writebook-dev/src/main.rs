//! `bin/dev`, everything it does and its help. Every command is a noun and a verb under it,
//! `bin/dev <noun> <verb>`.

mod lint;
mod review;
mod test;

use std::error::Error;

use usage::Subcommands;

use root::Dev;

#[expect(
    clippy::disallowed_methods,
    reason = "usage-derive's Cli expands to a map_or of its own"
)]
mod root {
    use usage::Cli;

    use super::Command;

    #[derive(Cli)]
    #[usage(
        about = "Everything for developing writebook-cli: linting and reviewing the code, and running the tests",
        bin = "bin/dev",
        version,
        after_help = "Each command's --help lists what it does."
    )]
    pub(super) struct Dev {
        #[usage(subcommand)]
        pub(super) command: Command,
    }
}

#[derive(Subcommands)]
enum Command {
    #[usage(help = "Lint and review the code")]
    Code {
        #[usage(subcommand)]
        command: Code,
    },
    #[usage(help = "Run the test suite")]
    Test {
        #[usage(subcommand)]
        command: Test,
    },
}

#[derive(Subcommands)]
enum Code {
    #[usage(help = "Check the formatting, run clippy with every warning as an error, and check the style rules")]
    Lint,
    #[usage(help = "Lint, then have two reviewers check the branch's diff against main's doc/review.md")]
    Review {
        #[usage(long, help = "Print the verdict without posting it on the pull request")]
        no_post: bool,
    },
}

#[derive(Subcommands)]
enum Test {
    #[usage(help = "Run every test with nextest, the live ones too when WRITEBOOK_LIVE_URL is set")]
    Run,
}

fn main() -> Result<(), Box<dyn Error>> {
    match Dev::parse().command {
        Command::Code { command: Code::Lint } => lint::run(),
        Command::Code {
            command: Code::Review { no_post },
        } => review::run(!no_post),
        Command::Test { command: Test::Run } => test::run(),
    }
}
