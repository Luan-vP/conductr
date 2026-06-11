use anyhow::Result;
use clap::Parser;

use conductr_dashboard_daemon::server::Daemon;

#[derive(Parser)]
#[command(name = "conductr-daemon", about = "conductr dashboard state daemon (v1 read-only)")]
struct Args {
    /// Override the Unix socket path.
    #[arg(long, env = "CONDUCTR_DAEMON_SOCKET")]
    socket: Option<std::path::PathBuf>,

    /// State refresh interval in seconds.
    #[arg(long, default_value = "30")]
    poll_secs: u64,

    /// Serve the web dashboard on this TCP port. Defaults to 12123.
    /// Pass --no-web to disable.
    #[arg(long, default_value = "12123")]
    web_port: u16,

    /// Disable the web dashboard TCP server.
    #[arg(long)]
    no_web: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();

    let socket = args.socket.unwrap_or_else(Daemon::default_socket_path);
    let mut daemon = Daemon::new(socket)
        .with_poll_interval(std::time::Duration::from_secs(args.poll_secs));
    if !args.no_web {
        daemon = daemon.with_web_port(args.web_port);
    }

    daemon.run().await
}
