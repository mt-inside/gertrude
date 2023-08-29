use futures::prelude::*;
use irc::client::prelude::*;
use nom::{
    bytes::complete::{is_a, is_not, tag},
    character::complete::alpha1,
    combinator::opt,
    multi::fold_many0,
    sequence::terminated,
    IResult,
};
use std::collections::HashMap;
use unicase::UniCase;

fn parse_many<'a, 'b>(
    i: &'a str,
    karma: &'b mut HashMap<UniCase<String>, i32>,
) -> IResult<&'a str, ()> {
    // TODO: nom-unicode. basically need a set of either: all token-admissable chars, or all non-token (space & punctuation)
    let token_chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz+-";
    let words = terminated(is_a(token_chars), opt(is_not(token_chars)));
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
            ()
        },
    );
    parser(i)
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    go("");
    go("no votes");
    go("--");
    go("bacon++");
    go("bacon++. Oh dear emacs crashed");
    go("Drivel about LISP. bacon++. Oh dear emacs crashed");
    go("Drivel about LISP. bacon++. Oh dear emacs crashed. Moat bacon++! This code rocks; mt++. Shame that lazy bb-- didn't do it.");

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
        println!("{:?}", message);
        if let Command::PRIVMSG(ref recipient, ref text) = message.command {
            if text.starts_with(client.current_nickname())
                || recipient.eq(client.current_nickname())
            {
                // TODO: more parsing, have a command syntax for her. Eg "karma foo"
                // gonna be the thing of try one compiler, then another if it fails (alt?) - or if-let?, and
                // within them, prefix? value? recognize?
                // Actually test this parser before you deploy it!

                let cur = format!("{:?}", karma);
                println!("Sending response to {:?}", message.response_target());
                client.send_privmsg(message.response_target().unwrap(), cur)?;
            } else {
                // TODO: error handling, but can't just ? it up because that (exceptionally) returns text, which doesn't live long enough
                let res = parse_many(text, &mut karma);
                println!("{:?}", res);
                println!("{:?}", karma);
            }
        }
    }

    Ok(())
}

fn go(s: &str) -> () {
    let mut k = HashMap::new();
    parse_many(s, &mut k).unwrap();
    println!("{:?}", k);
}
