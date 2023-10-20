/* TODO
 * - use dyn Trait for plugins
 * - optinoally run an ident server, attesting to who she is - doesn't seem relevant any more? Can't even see the ~ in irssi
 * - Plugins for
 *   - applying s/ regex to the last message, and printing the result, or "your regex doesn't work, oaf"
 *   - stock price
 *   - recording every spotify link send, with sender
 *   - bbc news article titles
 */

// Opt in to unstable Path::file_prefix, which is used in admin. Feature annotations have to be in the create root, ie here
#![feature(path_file_prefix)]

mod admin;
mod chatbot;
mod http_srv;
mod karma;
mod metrics;
mod plugins;

use clap::Parser;
use tokio::time::Duration;
use tokio_graceful_shutdown::{SubsystemHandle, Toplevel};
use tracing::*;
use tracing_subscriber::{filter, prelude::*};

pub static NAME: &str = env!("CARGO_BIN_NAME"); // clap only had a macro for crate name
pub static VERSION: &str = clap::crate_version!();

#[derive(Parser, Clone, Debug, Default)]
#[command(name = NAME, version = VERSION, about, author)] // about and author use default clap::crate_foo!(), which read from Cargo.toml
pub struct Args {
    /// IRC server to which to connect
    #[arg(short, long)]
    server: String,

    /// Channel to join
    #[arg(short, long)]
    channel: String,

    /// Nickname to use
    #[arg(short, long, default_value_t = NAME.to_owned())]
    nick: String,

    /// Bind address for the HTTP server (metrics, liveness, etc)
    #[arg(long, default_value_t = String::from("127.0.0.1:8080"))]
    http_addr: String,

    /// Path to file wither to load and whence to persist karma. Created if non-existant. No persistance if not provided. Path is not canonicalized, file handle not held open. Recommended extension is .binpb.
    ///
    /// https://protobuf.dev/programming-guides/techniques/#suffixes
    #[arg(long)]
    persist_path: Option<String>,

    /// Directory to watch for plugins
    #[arg(long)]
    plugin_dir: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let args = Args::parse();

    // Recall: foo=>tracing::Value; %foo=>fmt::Display; ?foo=>fmt::Debug
    tracing_subscriber::registry()
        .with(filter::Targets::new().with_default(Level::INFO).with_target(NAME, Level::TRACE))
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .init();

    let metrics = metrics::Metrics::new();
    let plugins = plugins::new_manager(args.plugin_dir.as_deref());
    let karma = karma::Karma::from_file(args.persist_path.as_deref(), metrics.clone());
    let srv = http_srv::HTTPSrv::new(args.http_addr.clone(), metrics.clone());
    let adm = admin::Admin::new(karma.clone(), plugins.clone());
    let bot = chatbot::Chatbot::new(args.clone(), karma.clone(), plugins.clone(), metrics.clone());

    Toplevel::new()
        .start("irc_client", move |subsys: SubsystemHandle| bot.lurk(subsys))
        .start("http_server", move |subsys: SubsystemHandle| srv.serve(subsys))
        .start("grpc_server", move |subsys: SubsystemHandle| adm.serve(subsys))
        .start("plugin_manager", move |subsys: SubsystemHandle| async move { plugins.watch(subsys) })
        .catch_signals()
        .handle_shutdown_requests(Duration::from_millis(5000))
        .await
        .map_err(Into::into)
}
