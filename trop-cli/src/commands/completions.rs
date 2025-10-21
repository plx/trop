//! Shell completion generation command.
//!
//! This module provides the `completions` command which generates shell completion
//! scripts for bash, zsh, fish, and PowerShell.

use crate::cli::Cli;
use crate::error::CliError;
use crate::utils::GlobalOptions;
use clap::{CommandFactory, Parser};
use clap_complete::{generate, Shell};
use std::io;

/// Binary name from Cargo.toml package name
const BIN_NAME: &str = env!("CARGO_PKG_NAME");

/// Generate shell completion scripts
#[derive(Parser)]
pub struct CompletionsCommand {
    /// Shell to generate completions for
    #[arg(value_enum)]
    pub shell: Shell,
}

impl CompletionsCommand {
    /// Execute the completions command.
    pub fn execute(&self, _global: &GlobalOptions) -> Result<(), CliError> {
        let mut cmd = Cli::command();
        let bin_name = BIN_NAME;

        eprintln!("# Generating {} completion script", self.shell);
        eprintln!("# Run the following command to enable completions:");

        match self.shell {
            Shell::Bash => {
                eprintln!(
                    "#   trop completions bash > ~/.local/share/bash-completion/completions/trop"
                );
                eprintln!("# Or source it directly in ~/.bashrc:");
                eprintln!("#   eval \"$(trop completions bash)\"");
            }
            Shell::Zsh => {
                eprintln!("#   trop completions zsh > ~/.zsh/completions/_trop");
                eprintln!("# Make sure ~/.zsh/completions is in your $fpath");
                eprintln!("# Or add to ~/.zshrc:");
                eprintln!("#   eval \"$(trop completions zsh)\"");
            }
            Shell::Fish => {
                eprintln!("#   trop completions fish > ~/.config/fish/completions/trop.fish");
                eprintln!("# Or add to config.fish:");
                eprintln!("#   trop completions fish | source");
            }
            Shell::PowerShell => {
                eprintln!("#   trop completions powershell > $PROFILE");
                eprintln!("# Or run:");
                eprintln!("#   trop completions powershell | Out-String | Invoke-Expression");
            }
            Shell::Elvish => {
                // Elvish included by default in clap_complete but no custom instructions needed
            }
            _ => {
                // Future shells added to clap_complete
            }
        }

        eprintln!();

        generate(self.shell, &mut cmd, bin_name, &mut io::stdout());

        Ok(())
    }
}
