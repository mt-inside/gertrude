use futures::prelude::*;
use irc::client::prelude::*;
use nom::{
    branch::alt,
    bytes::complete::{tag, take_till, take_till1},
    combinator::{eof, opt, value},
    multi::many0,
    sequence::{delimited, pair, preceded, tuple},
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
                    debug!(?message, "received");

                    if let Command::PRIVMSG(ref recipient, ref text) = message.command {
                        self.metrics.messages.with_label_values(&["privmsg"]).inc();

                        let nick = client.current_nickname();
                        if let Some(dm) = get_dm(nick, recipient, text) {
                            let from = message.source_nickname().unwrap();
                            let to = message.response_target().unwrap();

                            self.metrics.dms.with_label_values(&[from, to]).inc();

                            let res = parse_dm(dm, &self.karma);
                            match res {
                                Ok(resp) => {
                                    debug!(target = to, "Sending response");
                                    client.send_privmsg(to, resp)?;
                                },
                                Err(e) => {
                                    error!(?e, "Error parsing DM");
                                    client.send_privmsg(to, "unknown command / args")?;
                                },
                            }
                        } else {
                            let to = message.response_target().unwrap();

                            let res = parse_chat(text);
                            match res {
                                Ok(biases) => { debug!("Chat parsed ok");
                                    self.karma.bias_from(biases);
                                },
                                Err(e) => error!(?e, "Error parsing chat"),
                            }

                            // See if any of the plugins want to say anything
                            for res in self.plugins.handle_privmsg(text) {
                                match res {
                                    Ok(output) => client.send_privmsg(to, output)?,
                                    // TODO: don't iterate the plugins here, but make sure the errors contain plugin name etc
                                    Err(e) => error!(?e, "Plugin error"),
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
                    info!("Chatbot task got shutdown request");
                    client.send_privmsg(self.args.channel, "Killed!")?;
                    break
                },
            };
        }

        Ok(())
    }
}

fn word<'a>() -> impl FnMut(&'a str) -> IResult<&'a str, &'a str, nom::error::Error<&'a str>> {
    alt((delimited(tag("\""), take_till(|c| c == '"'), tag("\"")), alphanumeric1))
}

fn is_word_char(c: char) -> bool {
    is_alphanumeric(c) || c == '"' || c == '+' || c == '-'
}

fn parse_chat(text: &str) -> Result<Vec<(&str, i32)>, nom::Err<nom::error::Error<&str>>> {
    //          /me       -> current channel -> Message { tags: None, prefix: Some(Nickname("empty", _, _)), command: PRIVMSG("#ant.org", "\u{1}ACTION lol\u{1}") }
    // [ignore] /describe -> specific user   -> Message { tags: None, prefix: Some(Nickname("empty", _, _)), command: PRIVMSG("gertrude", "\u{1}ACTION lol\u{1}") }
    let mut action = delimited(
        nom::character::complete::char::<&str, nom::error::Error<&str>>('\x01'),
        preceded(tag("ACTION "), take_till(|c| c == '\x01')), // even if you just issue "/me", there's still a space after ACTION
        nom::character::complete::char('\x01'),
    );
    let mut karmic = tuple((alt((value(1, tag("hugs")), value(-1, tag("slaps")))), space1::<&str, nom::error::Error<&str>>, word(), eof));

    trace!(res = ?action(text), "action parser");
    if let Ok((_, cmd)) = action(text) {
        debug!(cmd, "ACTION");
        trace!(res = ?karmic(text), "karmic parser");
        if let Ok((rest, (bias, _, term, _))) = karmic(cmd) {
            // only one-word terms for now
            if rest.is_empty() {
                debug!(bias, term, "Karmic action");
                return Ok(vec![(term, bias)]);
            }
        }
    }

    let mut words = delimited(
        take_till(is_word_char),
        alt((pair(word(), alt((value(1, tag("++")), value(-1, tag("--"))))), value(("", 0), word()))),
        take_till(is_word_char),
    );
    trace!(res = ?words(text), "words parser (invocation 1)");
    let mut parser = many0(words);
    trace!(res = ?parser(text), "many0 parser");
    parser(text).map(|(_rest, vec)| vec.into_iter().filter(|(_, n)| n != &0).collect())
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

fn parse_dm<'a>(text: &'a str, karma: &Karma) -> Result<String, nom::Err<nom::error::Error<&'a str>>> {
    let mut p_karma = tuple((tag("karma"), opt(space1::<&str, nom::error::Error<&str>>), opt(word())));

    p_karma(text).map(|(_rest, (_, _, arg))| match arg {
        None => {
            info!("Command: karma all");
            format!("{}", karma)
        }
        Some(token) => {
            info!(token, "Command: karma");
            format!("{}", karma.get(token))
        }
    })
}

#[cfg(test)]
mod tests {
    use maplit::hashmap;

    //use tracing_test::traced_test;
    use super::*;

    #[test]
    fn test_parse_chat() {
        let cases = [
            ("", vec![]),
            ("no votes", vec![]),
            ("--", vec![]),
            ("bacon++", vec![("bacon", 1)]),
            ("bacon++. Oh dear emacs crashed", vec![("bacon", 1)]),
            ("Drivel about LISP. bacon++. Oh dear emacs crashed", vec![("bacon", 1)]),
            (
                "Drivel about LISP. bacon++. Oh dear emacs crashed. Moar bacon++! This code rocks; mt++. Shame that lazy bb-- didn't do it.",
                vec![("bacon", 1), ("bacon", 1), ("mt", 1), ("bb", -1)],
            ),
            ("BaCoN++ bAcOn++ bacon++ BACON++", vec![("BaCoN", 1), ("bAcOn", 1), ("bacon", 1), ("BACON", 1)]),
            ("blɸwback++", vec![("blɸwback", 1)]),
            ("foo 💩++", vec![]), // emoji aren't alphanumeric. Need a printable-non-space
            ("💩++", vec![]),     // emoji aren't alphanumeric. Need a printable-non-space
            ("\"foo bar\"++", vec![("foo bar", 1)]),
            ("foo \"foo bar\"++ bar", vec![("foo bar", 1)]),
            ("foo++ \"foo bar\"++ bar++", vec![("foo", 1), ("foo bar", 1), ("bar", 1)]),
            ("\"💩\"++", vec![("💩", 1)]), // this parser works differently...
        ];

        for case in cases {
            let res = parse_chat(case.0);
            assert!(res.is_ok(), "parse failed");
            assert_eq!(res.unwrap(), case.1);
        }
    }

    #[test]
    fn test_karma_ingest() {
        let cases = [
            (vec![vec![]], hashmap![]),
            (vec![vec![("bacon", 1)]], hashmap!["bacon" => 1]),
            (vec![vec![("bacon", 1), ("bacon", 1), ("mt", 1), ("bb", -1)]], hashmap!["bacon" => 2, "mt" => 1, "bb" => -1]),
            (vec![vec![("BaCoN", 1), ("bAcOn", 1), ("bacon", 1), ("BACON", 1)]], hashmap!["BaCoN" => 4]),
            (vec![vec![("blɸwback", 1)]], hashmap!["blɸwback" => 1]),
            (vec![vec![("foo bar", 1)]], hashmap!["foo bar" => 1]),
            (vec![vec![("foo", 1), ("foo bar", 1), ("bar", 1)]], hashmap!["foo bar" => 1, "foo" => 1, "bar" => 1]),
            (vec![vec![("💩", 1)]], hashmap!["💩" => 1]),
            (vec![vec![("bacon", 1)], vec![("bacon", 1)]], hashmap!["bacon" => 2]),
            (vec![vec![("bacon", 1)], vec![("bacon", -1)]], hashmap!["bacon" => 0]),
        ];

        for case in cases {
            let k = Karma::new(Metrics::new());
            for t in case.0 {
                k.bias_from(t);
            }
            assert_eq!(k, case.1);
        }
    }

    #[test]
    fn test_parse_dm_karma() {
        let k = Karma::from(hashmap![
            "bacon" => 1,
            "blɸwback" => -1,
            "rust" => 666,
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
            let res = parse_dm(case.0, &k);
            assert!(res.is_ok(), "parse failed");
            assert_eq!(res.unwrap(), case.1);
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
