use futures::prelude::*;
use irc::client::prelude::*;
use nom::{
    bytes::complete::{is_a, is_not, tag},
    character::complete::{alpha1, space1},
    combinator::opt,
    multi::fold_many0,
    sequence::{terminated, tuple},
    IResult,
};
use std::collections::HashMap;
use unicase::UniCase;

const TOKEN_CHARS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz+-";

fn parse_many<'a>(i: &'a str, karma: &mut HashMap<UniCase<String>, i32>) -> IResult<&'a str, ()> {
    // TODO: nom-unicode. basically need a set of either: all token-admissable chars, or all non-token (space & punctuation)
    let words = terminated(is_a(TOKEN_CHARS), opt(is_not(TOKEN_CHARS)));
    let mut upvote = opt(terminated(
        alpha1::<&str, nom::error::Error<&str>>,
        tag("++"),
    ));
    let mut downvote = opt(terminated(
        alpha1::<&str, nom::error::Error<&str>>,
        tag("--"),
    ));
    let mut parser = fold_many0(
        words,
        || (),
        |(), item| {
            if let Ok((_, Some(term))) = upvote(item) {
                let count = karma.entry(UniCase::new(term.to_owned())).or_insert(0);
                *count += 1;
            } else if let Ok((_, Some(term))) = downvote(item) {
                let count = karma.entry(UniCase::new(term.to_owned())).or_insert(0);
                *count -= 1;
            }
        },
    );
    parser(i)
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    go_lurk().await?;

    Ok(())
}

async fn go_lurk() -> Result<(), anyhow::Error> {
    let config = Config {
        nickname: Some("gertrude".to_owned()),
        server: Some("irc.z.je".to_owned()),
        channels: vec!["#ant.org".to_owned()],
        ..Default::default()
    };
    let mut client = Client::from_config(config).await?;
    client.identify()?;

    let mut karma = HashMap::new();
    let mut stream = client.stream()?;

    while let Some(message) = stream.next().await.transpose()? {
        println!("Message: {:?}", message);
        let nick = client.current_nickname();
        if let Command::PRIVMSG(ref recipient, ref text) = message.command {
            if let Some(msg) = get_dm(nick, recipient, text) {
                let resp = get_resp(msg, &karma);
                println!("Sending response to {:?}", message.response_target());
                client.send_privmsg(message.response_target().unwrap(), resp)?;
            } else {
                // TODO: error handling, but can't just ? it up because that (exceptionally) returns text, which doesn't live long enough
                let res = parse_many(text, &mut karma);
                println!("Token parsing result: {:?}", res);
                println!("Karma now: {:?}", karma);
            }
        }
    }

    Ok(())
}

fn get_resp(text: &str, karma: &HashMap<UniCase<String>, i32>) -> String {
    let mut p_karma = tuple((
        tag("karma"),
        opt(space1::<&str, nom::error::Error<&str>>),
        opt(alpha1), // TODO: use letters, derive +/- version from orig
    ));
    if let Ok((_rest, (_, _, arg))) = p_karma(text) {
        match arg {
            None => {
                println!("Command: karma all");
                format!("{:?}", karma)
            }
            Some(token) => {
                println!("Command: karma {}", token);
                format!(
                    "{}",
                    karma.get(&UniCase::new(token.to_owned())).unwrap_or(&0)
                )
            }
        }
    } else {
        "unknown command / args".to_owned()
    }
}

// fn strip_nick<'a, 'b>(s: &'a str, nick: &'b str) -> &'a str {
//     match s.strip_prefix(nick) {
//         Some(msg) => msg.trim_start_matches([':', '>']).trim_start(),
//         None => s,
//     }
// }
fn get_dm<'a>(nick: &str, target: &str, s: &'a str) -> Option<&'a str> {
    let mut p_dm = tuple((
        tag::<&str, &str, nom::error::Error<&str>>(nick),
        is_not(TOKEN_CHARS),
    ));

    println!("Is it a DM? {:?}", p_dm(s));

    if target.eq(nick) {
        Some(s)
    } else if let Ok((rest, _)) = p_dm(s) {
        Some(rest)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maplit::hashmap;
    use std::collections::HashMap;

    fn fix_map_types(m: HashMap<&str, i32>) -> HashMap<UniCase<String>, i32> {
        let mut exp = HashMap::new();
        exp.extend(
            m.into_iter()
                .map(move |(k, v)| (UniCase::new(k.to_owned()), v)),
        );
        exp
    }

    #[test]
    fn test_parse_many() {
        let cases = [
            ("", hashmap![]),
            ("no votes", hashmap![]),
            ("--", hashmap![]),
            ("bacon++", hashmap!["bacon"=> 1]),
            ("bacon++. Oh dear emacs crashed", hashmap!["bacon"=>1]),
            ("Drivel about LISP. bacon++. Oh dear emacs crashed", hashmap!["bacon"=>1]),
            ("Drivel about LISP. bacon++. Oh dear emacs crashed. Moat bacon++! This code rocks; mt++. Shame that lazy bb-- didn't do it.", hashmap!["bacon"=>2, "mt" => 1, "bb" =>-1]),
        ];

        for case in cases {
            let mut k = HashMap::new();
            assert_eq!(parse_many(case.0, &mut k), Ok(("", ())));
            assert_eq!(k, fix_map_types(case.1));
        }
    }

    #[test]
    fn test_get_resp() {
        let k = fix_map_types(hashmap![
            "bacon" => 1,
            "rust" => 666,
            "LISP" => -666,
        ]);
        let k_rendered = format!("{:?}", k);
        let cases = [("karma", k_rendered), ("karma bacon", "1".to_owned())];

        for case in cases {
            assert_eq!(get_resp(case.0, &k.clone()), case.1);
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
            (
                "gertie",
                "gertie",
                "gertie, foo bar",
                Some("gertie, foo bar"),
            ),
        ];

        for case in cases {
            assert_eq!(get_dm(case.0, case.1, case.2), case.3);
        }
    }
}
