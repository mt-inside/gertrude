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

fn parse_many(i: &str) -> IResult<&str, HashMap<String, i32>> {
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
    let mut parser = fold_many0(words, HashMap::new, |mut m, item| {
        if let Ok((_, Some(term))) = upvote(item) {
            let count = m.entry(term.to_owned()).or_insert(0);
            *count += 1;
        } else if let Ok((_, Some(term))) = downvote(item) {
            let count = m.entry(term.to_owned()).or_insert(0);
            *count -= 1;
        }
        m
    });
    parser(i)
}

#[tokio::main]
async fn main() -> Result<(), irc::error::Error> {
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

async fn go_lurk() -> Result<(), irc::error::Error> {
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
        if let Command::PRIVMSG(_recipient, text) = message.command {
            let news = parse_many(&text).unwrap().1;
            println!("{:?}", news);
            karma.extend(news);
            println!("{:?}", karma);
        }
    }

    Ok(())
}

fn go(s: &str) -> () {
    let k = parse_many(s).unwrap();
    println!("{:?}", k);
}
