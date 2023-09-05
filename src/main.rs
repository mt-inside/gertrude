/* TODO
 * - plugin arch using WASM. Plugins for
 *   - stock price
 *   - recording every spotify link send, with sender
 */

mod admin;
mod chatbot;
mod http_srv;
pub mod karma;
pub mod metrics;

use clap::Parser;
use tokio::time::Duration;
use tokio_graceful_shutdown::{SubsystemHandle, Toplevel};
use tracing::*;
use tracing_subscriber::{filter, prelude::*};

pub static NAME: &str = env!("CARGO_BIN_NAME"); // has hypens; CARGO_CRATE_NAME for underscores
pub static VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser, Clone, Debug, Default)]
#[command(name = NAME)]
#[command(author = "Matt Turner")]
#[command(version = VERSION)]
#[command(about = format!("botten {}", NAME), long_about = None)]
pub struct Args {
    #[arg(short, long)]
    server: String,
    #[arg(short, long)]
    channel: String,
    #[arg(short, long, default_value_t = NAME.to_owned())]
    nick: String,
    #[arg(long, default_value_t = String::from("127.0.0.1:8080"))]
    http_addr: String,
    #[arg(long)]
    plugin_dir: String,
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
    let karma = karma::Karma::new(metrics.clone());
    let srv = http_srv::HTTPSrv::new(args.http_addr.clone(), metrics.clone());
    let adm = admin::Admin::new(karma.clone());
    let bot = chatbot::Chatbot::new(args.clone(), karma.clone(), metrics.clone());

    Toplevel::new()
        .start("irc_client", move |subsys: SubsystemHandle| bot.lurk(subsys))
        .start("http_server", move |subsys: SubsystemHandle| srv.serve(subsys))
        .start("grpc_server", move |subsys: SubsystemHandle| adm.serve(subsys))
        .catch_signals()
        .handle_shutdown_requests(Duration::from_millis(5000))
        .await
        .map_err(Into::into)
}
