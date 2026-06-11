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

    /// Serve the web dashboard on this TCP port (e.g. --web-port 7777).
    #[arg(long)]
    web_port: Option<u16>,
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
    if let Some(port) = args.web_port {
        daemon = daemon.with_web_port(port);
    }

    daemon.run().await
}
