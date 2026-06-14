#[cfg(feature = "tmux")]
pub mod tmux;

#[cfg(feature = "crontab")]
pub mod crontab;

#[cfg(feature = "beads")]
pub mod beads;

#[cfg(feature = "notion")]
pub mod notion;

#[cfg(feature = "gh-cli")]
pub mod gh_cli;

#[cfg(feature = "mail-fs")]
pub mod mail_fs;

#[cfg(feature = "mail-github")]
pub mod mail_github;

#[cfg(feature = "mock")]
pub mod mock;

#[cfg(feature = "ollama")]
pub mod ollama;

#[cfg(feature = "llamacpp")]
pub mod llamacpp;

#[cfg(feature = "local-ci")]
pub mod local_ci;

#[cfg(feature = "gcal")]
pub mod gcal;
