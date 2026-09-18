//! windows-board: a Windows-first board plugin for Herdr.

mod commands;
mod herdr_cli;
mod state;
mod tui;

#[cfg(test)]
mod test_support;

use clap::{Parser, Subcommand};

/// Windows-first board plugin for Herdr.
#[derive(Parser)]
#[command(name = "windows-board", version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Launch the interactive terminal user interface.
    Tui,
    /// Open the board (summon the TUI) or, with a target, open that target in
    /// the Windows default handler.
    Open {
        /// Optional file, path, or URL to open.
        target: Option<String>,
    },
    /// Manage board cards.
    #[command(subcommand)]
    Card(CardCommand),
    /// Check environment prerequisites and report status.
    Doctor,
}

#[derive(Subcommand)]
pub enum CardCommand {
    /// Add a new card.
    Add {
        /// Card title.
        title: String,
        /// Column to place the card in.
        #[arg(long, default_value = "todo")]
        column: String,
    },
        /// Add a card from the currently focused/invoking pane.
        AddFromPane {
            /// Optional card title; defaults to the pane's title (or cwd).
            title: Option<String>,
            /// Column to place the card in.
            #[arg(long, default_value = "todo")]
            column: String,
        },
        /// Focus the pane linked to a card.
        Focus {
            /// Card id.
            id: String,
        },
        /// Clear stale pane/agent associations on cards.
        Refresh,
        /// List all cards.
        List,
    /// Move a card to another column.
    Move {
        /// Card id.
        id: String,
        /// Destination column.
        column: String,
    },
    /// Remove a card.
    Remove {
        /// Card id.
        id: String,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Tui => tui::run(),
        Command::Open { target } => commands::open(target),
        Command::Card(cmd) => commands::card(cmd),
        Command::Doctor => commands::doctor(),
    }
}