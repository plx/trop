//! Shell auto-detection against the real process environment.
//!
//! This binary holds a single test because it mutates the process
//! environment (`SHELL` and the shell version variables).

use std::env;

use trop::output::ShellType;

/// `SHELL` used to be classified by substring over the whole path, so
/// `/home/selfish/bin/bash` produced fish syntax, and an unknown or missing
/// shell silently produced bash syntax instead of an error.
#[test]
fn shell_detection_uses_the_final_path_component_and_fails_closed() {
    for variable in ["ZSH_VERSION", "FISH_VERSION", "PSModulePath"] {
        env::remove_var(variable);
    }

    for (shell, expected) in [
        ("/home/selfish/bin/bash", ShellType::Bash),
        ("/opt/powershell-addons/bin/bash", ShellType::Bash),
        ("/home/zshfan/bin/fish", ShellType::Fish),
        ("/usr/bin/pwsh", ShellType::PowerShell),
        ("/bin/zsh", ShellType::Zsh),
    ] {
        env::set_var("SHELL", shell);
        assert_eq!(ShellType::detect().unwrap(), expected, "SHELL={shell}");
    }

    for shell in ["/bin/tcsh", "", "/usr/bin/"] {
        env::set_var("SHELL", shell);
        let error = ShellType::detect().unwrap_err().to_string();
        assert!(error.contains("--shell"), "SHELL={shell}: {error}");
    }

    env::remove_var("SHELL");
    assert!(ShellType::detect().is_err());

    env::set_var("FISH_VERSION", "3.7.1");
    assert_eq!(ShellType::detect().unwrap(), ShellType::Fish);
    env::set_var("ZSH_VERSION", "5.9");
    assert_eq!(ShellType::detect().unwrap(), ShellType::Zsh);
}
