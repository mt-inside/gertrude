use futures::prelude::*;
use irc::client::prelude::*;
use nom::{
    bytes::complete::{tag, take_till, take_till1, take_while1},
    combinator::opt,
    multi::fold_many0,
    sequence::{delimited, terminated, tuple},
    IResult,
};
use nom_unicode::{
    complete::{alphanumeric1, space1},
    is_alphanumeric,
};
use tokio_graceful_shutdown::SubsystemHandle;
use tracing::*;

use super::Args;
use crate::{karma::Karma, metrics::Metrics, plugins::WasmPlugins};

pub struct Chatbot {
    args: Args,
    karma: Karma,
    plugins: WasmPlugins,
    metrics: Metrics,
}

impl Chatbot {
    pub fn new(args: Args, karma: Karma, plugins: WasmPlugins, metrics: Metrics) -> Self {
        Self { args, karma, plugins, metrics }
    }

    pub async fn lurk(self, subsys: SubsystemHandle) -> Result<(), anyhow::Error> {
        let config = Config {
            nickname: Some(self.args.nick.clone()),
            server: Some(self.args.server.clone()),
            channels: vec![self.args.channel.clone()],
            version: Some(format!("{} {} {}/{}", crate::NAME, crate::VERSION, std::env::consts::OS, std::env::consts::ARCH,)),
            source: Some(env!("CARGO_PKG_REPOSITORY").to_owned()),
            user_info: Some(format!("Jag känner en bot, hon heter {0}, {0} heter hon", crate::NAME,)),
            ..Default::default()
        };
        let mut client = Client::from_config(config).await?;
        client.identify()?;

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

                            let resp = parse_dm(dm, &self.karma);

                            debug!(target = to, "Sending response");
                            client.send_privmsg(to, resp)?;
                        } else {
                            // TODO: error handling, but can't just ? it up because that (exceptionally) returns text, which doesn't live long enough
                             let res = parse_chat(text, &self.karma);
                             debug!(?res, "Chat parsing result");

                            // See if any of the plugins want to say anything
                            for res in self.plugins.handle_privmsg(text) {
                                match res {
                                    Ok(output) => client.send_privmsg(message.response_target().unwrap(), output)?,
                                    Err(e) => error!(?e, "WASM"), // TODO also send to channel eg
                                                                  // "plugins error <plugin name>
                                                                  // ..."
                                }
                            }
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

fn parse_chat<'a>(text: &'a str, karma: &Karma) -> IResult<&'a str, ()> {
    let words = delimited(take_till(is_alphanumeric), take_while1(is_alnumvote), take_till(is_alphanumeric));
    let mut upvote = opt(terminated(alphanumeric1::<&str, nom::error::Error<&str>>, tag("++")));
    let mut downvote = opt(terminated(alphanumeric1::<&str, nom::error::Error<&str>>, tag("--")));
    let mut parser = fold_many0(
        words,
        || (),
        |(), item| {
            if let Ok((_, Some(term))) = upvote(item) {
                karma.bias(term, 1);
            } else if let Ok((_, Some(term))) = downvote(item) {
                karma.bias(term, -1);
            }
        },
    );
    parser(text)
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

fn parse_dm(text: &str, karma: &Karma) -> String {
    let mut p_karma = tuple((tag("karma"), opt(space1::<&str, nom::error::Error<&str>>), opt(alphanumeric1)));

    if let Ok((_rest, (_, _, arg))) = p_karma(text) {
        match arg {
            None => {
                info!("Command: karma all");
                format!("{}", karma)
            }
            Some(token) => {
                info!(token, "Command: karma");
                format!("{}", karma.get(token))
            }
        }
    } else {
        "unknown command / args".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use maplit::hashmap;
    use unicase::UniCase;

    use super::*;

    #[test]
    fn test_parse_chat() {
        let cases = [
            ("", hashmap![]),
            ("no votes", hashmap![]),
            ("--", hashmap![]),
            ("bacon++", hashmap!["bacon" => 1]),
            ("bacon++. Oh dear emacs crashed", hashmap!["bacon" => 1]),
            ("Drivel about LISP. bacon++. Oh dear emacs crashed", hashmap!["bacon" => 1]),
            (
                "Drivel about LISP. bacon++. Oh dear emacs crashed. Moat bacon++! This code rocks; mt++. Shame that lazy bb-- didn't do it.",
                hashmap!["bacon" => 2, "mt" => 1, "bb" => -1],
            ),
            ("blɸwback++", hashmap!["blɸwback"=> 1]),
            ("foo 💩++", hashmap![]), // emoji aren't alphanumeric. Need a printable-non-space
            ("💩++", hashmap![]),     // emoji aren't alphanumeric. Need a printable-non-space
        ];

        for case in cases {
            let k = Karma::new(Metrics::new());
            let res = parse_chat(case.0, &k);
            assert!(res.is_ok(), "parse failed");
            assert_eq!(k, case.1);
        }
    }

    #[test]
    fn test_parse_dm_karma() {
        let k = Karma::from(hashmap![
            "bacon"=> 1,
            "blɸwback"=> -1,
            "rust"=> 666,
            "LISP" => -666,
        ]);
        let k_rendered = format!("{}", k);

        let cases = [
            ("karma", k_rendered.as_str()),
            ("karma bacon", "1"),
            ("karma BaCoN", "1"),
            ("karma lisp", "-666"),
            ("karma blɸwback", "-1"),
        ];

        for case in cases {
            let resp = parse_dm(case.0, &k);
            assert_eq!(resp, case.1);
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
