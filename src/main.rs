/* TODO
 * - metrics other interesting things like message count/rate, and of which are @gertie and actual dms
 * - admin command should be out-of-band: grpc interface I can hit (leave grpcurl scripts over loopback in repo)
 */

mod metrics;

use std::collections::HashMap;

use clap::Parser;
use futures::prelude::*;
use irc::client::prelude::*;
use maplit::hashmap;
use nom::{
    bytes::complete::{tag, take_till1, take_while1},
    combinator::opt,
    multi::fold_many0,
    sequence::{terminated, tuple},
};
use nom_unicode::{
    complete::{alphanumeric1, digit1, space1},
    is_alphanumeric,
};
use tokio::time::Duration;
use tokio_graceful_shutdown::{SubsystemHandle, Toplevel};
use tracing::*;
use tracing_subscriber::{filter, prelude::*};
use unicase::UniCase;

pub static NAME: &str = env!("CARGO_BIN_NAME"); // has hypens; CARGO_CRATE_NAME for underscores
pub static VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser, Clone, Debug, Default)]
#[command(name = NAME)]
#[command(author = "Matt Turner")]
#[command(version = VERSION)]
#[command(about = format!("botten {}", NAME), long_about = None)]
struct Args {
    #[arg(short, long)]
    server: String,
    #[arg(short, long)]
    channel: String,
    #[arg(short, long, default_value_t = NAME.to_owned())]
    nick: String,
    #[arg(long, default_value_t = String::from("127.0.0.1:8080"))]
    http_addr: String,
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let args = Args::parse();

    // Recall: foo=>tracing::Value; %foo=>fmt::Display; ?foo=>fmt::Debug
    tracing_subscriber::registry()
        .with(filter::Targets::new().with_default(Level::INFO).with_target(NAME, Level::TRACE))
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .init();

    // I think metrics is an Arc (due to all the stuff in it being Arc?) TODO: make args an Arc rather than deriving clone
    let metrics = metrics::Metrics::new();
    let srv = metrics::HTTPSrv::new(args.http_addr.clone(), metrics.clone());
    let bot = Chatbot::new(args.clone(), metrics.clone());

    Toplevel::new()
        .start("irc_client", move |subsys: SubsystemHandle| bot.lurk(subsys))
        .start("http_server", move |subsys: SubsystemHandle| srv.serve(subsys))
        .catch_signals()
        .handle_shutdown_requests(Duration::from_millis(5000))
        .await
        .map_err(Into::into)
}

struct Chatbot {
    args: Args,
    metrics: metrics::Metrics,
}

impl Chatbot {
    fn new(args: Args, metrics: metrics::Metrics) -> Self {
        Self { args, metrics }
    }

    async fn lurk(self, subsys: SubsystemHandle) -> Result<(), anyhow::Error> {
        let config = Config {
            nickname: Some(self.args.nick.clone()),
            server: Some(self.args.server.clone()),
            channels: vec![self.args.channel.clone()],
            version: Some(format!("{} {} {}/{}", NAME, VERSION, std::env::consts::OS, std::env::consts::ARCH,)),
            source: Some(env!("CARGO_PKG_REPOSITORY").to_owned()),
            user_info: Some(format!("Jag känner en bot, hon heter {0}, {0} heter hon", NAME,)),
            ..Default::default()
        };
        let mut client = Client::from_config(config).await?;
        client.identify()?;

        // TODO: need a karma type that wraps the map and updates metrics, logs, when set. Can also impl Display, and eg sort output by value
        let mut karma = HashMap::new();
        let mut stream = client.stream()?;

        loop {
            tokio::select! {
                Some(Ok(message)) = stream.next() => {
                    info!(?message, "received");

                    if let Command::PRIVMSG(ref recipient, ref text) = message.command {
                        self.metrics.messages.with_label_values(&["privmsg"]).inc();

                        let nick = client.current_nickname();
                        if let Some(dm) = get_dm(nick, recipient, text) {
                            let from = message.source_nickname().unwrap();
                            let to = message.response_target().unwrap();

                            self.metrics.dms.with_label_values(&[from, to]).inc();

                            let (news, resp) = parse_dm(dm, &karma);
                            karma.extend(news);

                            debug!(target = to, "Sending response");
                            client.send_privmsg(to, resp)?;
                        } else {
                            // TODO: error handling, but can't just ? it up because that (exceptionally) returns text, which doesn't live long enough
                            let news = parse_chat(text, &karma);
                            karma.extend(news);
                        }

                        // TODO: only print if it changed. If we get news here (needed for below), we can just check for non-empty set
                        info!(?karma, "Karma");
                        // TODO: only the news
                        for (k,v) in &karma {
                            self.metrics.karma.with_label_values(&[k.as_str()]).set(*v as f64);
                        }
                    } else if let Command::PING(ref srv1, ref _srv2) = message.command {
                        self.metrics.messages.with_label_values(&["ping"]).inc();
                        self.metrics.pings.with_label_values(&[srv1]).inc();
                    } else if let Command::PONG(ref srv1, ref _srv2) = message.command {
                        self.metrics.messages.with_label_values(&["pong"]).inc();
                        self.metrics.pongs.with_label_values(&[srv1]).inc();
                    }
                },
                _ = subsys.on_shutdown_requested() => {
                    info!("Bot task got shutdown request");
                    client.send_privmsg(self.args.channel, "Killed!")?;
                    break
                },
            };
        }

        Ok(())
    }
}

fn is_alnumvote(c: char) -> bool {
    is_alphanumeric(c) || c == '+' || c == '-'
}

fn parse_chat(text: &str, karma: &HashMap<UniCase<String>, i32>) -> HashMap<UniCase<String>, i32> {
    let words = terminated(take_while1(is_alnumvote), opt::<&str, &str, nom::error::Error<&str>, _>(take_till1(is_alphanumeric))); // opt(not(alphanumeric1)) doesn't work
    let mut upvote = opt(terminated(alphanumeric1::<&str, nom::error::Error<&str>>, tag("++")));
    let mut downvote = opt(terminated(alphanumeric1::<&str, nom::error::Error<&str>>, tag("--")));
    let mut parser = fold_many0(words, HashMap::new, |mut new_karma, item| {
        if let Ok((_, Some(term))) = upvote(item) {
            let old_count = karma.get(&UniCase::new(term.to_owned())).unwrap_or(&0);
            let new_count = new_karma.entry(UniCase::new(term.to_owned())).or_insert(*old_count);
            *new_count += 1;
        } else if let Ok((_, Some(term))) = downvote(item) {
            let old_count = karma.get(&UniCase::new(term.to_owned())).unwrap_or(&0);
            let new_count = new_karma.entry(UniCase::new(term.to_owned())).or_insert(*old_count);
            *new_count -= 1;
        }
        new_karma
    });
    let res = parser(text);
    debug!(?res, "Chat line parsing complete");

    // TODO: bad assumption. Believe it will blow up on "^<poop>$"
    res.unwrap().1
}

fn get_dm<'a>(nick: &str, target: &str, s: &'a str) -> Option<&'a str> {
    let mut p_dm = tuple((tag::<&str, &str, nom::error::Error<&str>>(nick), take_till1(is_alphanumeric)));

    debug!(parse_result = ?p_dm(s), "Is it a DM?");

    if target.eq(nick) {
        Some(s)
    } else if let Ok((rest, _)) = p_dm(s) {
        Some(rest)
    } else {
        None
    }
}

fn parse_dm(text: &str, karma: &HashMap<UniCase<String>, i32>) -> (HashMap<UniCase<String>, i32>, String) {
    let mut p_karma = tuple((tag("karma"), opt(space1::<&str, nom::error::Error<&str>>), opt(alphanumeric1)));
    let mut p_admin = tuple((
        tag("mattisskill"),
        space1::<&str, nom::error::Error<&str>>,
        tag("set"),
        space1::<&str, nom::error::Error<&str>>,
        alphanumeric1,
        space1::<&str, nom::error::Error<&str>>,
        opt(tag("-")),
        digit1,
    ));

    if let Ok((_rest, (_, _, arg))) = p_karma(text) {
        match arg {
            None => {
                info!("Command: karma all");
                (hashmap![], format!("{:?}", karma))
            }
            Some(token) => {
                info!(token, "Command: karma");
                (hashmap![], format!("{}", karma.get(&UniCase::new(token.to_owned())).unwrap_or(&0)))
            }
        }
    } else if let Ok((_rest, (_, _, _, _, token, _, sign, val))) = p_admin(text) {
        let new_count = val.parse::<i32>().unwrap() * if sign == Some("-") { -1 } else { 1 };
        info!(token, new_count, "Command: mattisskill");
        (hashmap![UniCase::new(token.to_owned()) => new_count], format!("{} now {}", token, new_count))
    } else {
        (hashmap![], "unknown command / args".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use maplit::hashmap;

    use super::*;

    fn fix_map_types(m: HashMap<&str, i32>) -> HashMap<UniCase<String>, i32> {
        let mut exp = HashMap::new();
        exp.extend(m.into_iter().map(move |(k, v)| (UniCase::new(k.to_owned()), v)));
        exp
    }

    #[test]
    fn test_parse_chat() {
        let cases = [
            ("", hashmap![]),
            ("no votes", hashmap![]),
            ("--", hashmap![]),
            ("bacon++", hashmap!["bacon"=> 1]),
            ("bacon++. Oh dear emacs crashed", hashmap!["bacon"=>1]),
            ("Drivel about LISP. bacon++. Oh dear emacs crashed", hashmap!["bacon"=>1]),
            (
                "Drivel about LISP. bacon++. Oh dear emacs crashed. Moat bacon++! This code rocks; mt++. Shame that lazy bb-- didn't do it.",
                hashmap!["bacon"=>2, "mt" => 1, "bb" =>-1],
            ),
            ("blɸwback++", hashmap!["blɸwback"=> 1]),
            ("foo 💩++", hashmap![]), // emoji aren't alphanumeric. Need a printable-non-space
                                      // "💩++" will fail, because TODO we need to strip
                                      // non-alphanumeric from the start, else the words splitter
                                      // won't work
        ];

        for case in cases {
            let k = HashMap::new();
            assert_eq!(parse_chat(case.0, &k), fix_map_types(case.1));
        }
    }

    #[test]
    fn test_parse_dm_karma() {
        let k = fix_map_types(hashmap![
            "bacon" => 1,
            "blɸwback" => -1,
            "rust" => 666,
            "LISP" => -666,
        ]);
        let k_rendered = format!("{:?}", k);
        let cases = [
            ("karma", k_rendered.as_str()),
            ("karma bacon", "1"),
            ("karma BaCoN", "1"),
            ("karma lisp", "-666"),
            ("karma blɸwback", "-1"),
        ];

        for case in cases {
            let (_deltas, resp) = parse_dm(case.0, &k);
            assert_eq!(resp, case.1);
        }
    }

    #[test]
    fn test_parse_dm_admin() {
        let k = fix_map_types(hashmap![
            "bacon" => 1,
            "blɸwback" => -1,
            "rust" => 666,
            "LISP" => -666,
        ]);
        let cases = [
            ("mattisskill set rust 612", "rust now 612", hashmap!["rust" => 612]),
            ("mattisskill set new 42", "new now 42", hashmap!["new" => 42]),
            ("mattisskill set newer -42", "newer now -42", hashmap!["newer" => -42]),
        ];

        for case in cases {
            let (news, resp) = parse_dm(case.0, &k);
            assert_eq!(resp, case.1);
            assert_eq!(news, fix_map_types(case.2));
        }
    }

    #[test]
    fn test_get_dm() {
        let cases = [
            ("gertie", "#chan", "foo bar", None),
            ("gertie", "#chan", "gertie foo bar", Some("foo bar")),
            ("gertie", "#chan", "gertie: foo bar", Some("foo bar")),
            ("gertie", "#chan", "gertie> foo bar", Some("foo bar")),
            ("gertie", "#chan", "gertie, foo bar", Some("foo bar")),
            ("gertie", "gertie", "foo bar", Some("foo bar")),
            ("gertie", "gertie", "gertie, foo bar", Some("gertie, foo bar")),
        ];

        for case in cases {
            assert_eq!(get_dm(case.0, case.1, case.2), case.3);
        }
    }
}
