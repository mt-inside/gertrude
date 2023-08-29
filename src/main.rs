use nom::{
    bytes::complete::{is_a, is_not, tag},
    character::complete::alpha1,
    combinator::opt,
    multi::fold_many0,
    sequence::terminated,
    IResult,
};
use std::collections::HashMap;

fn parse_many(i: &str) -> IResult<&str, HashMap<&str, i32>> {
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
    let mut parser = fold_many0(words, HashMap::new, move |mut m: HashMap<_, _>, item| {
        if let Ok((_, Some(term))) = upvote(item) {
            let count = m.entry(term).or_insert(0);
            *count += 1;
        } else if let Ok((_, Some(term))) = downvote(item) {
            let count = m.entry(term).or_insert(0);
            *count -= 1;
        }
        m
    });
    parser(i)
}

fn main() {
    go("");
    go("no votes");
    go("--");
    go("bacon++");
    go("bacon++. Oh dear emacs crashed");
    go("Drivel about LISP. bacon++. Oh dear emacs crashed");
    go("Drivel about LISP. bacon++. Oh dear emacs crashed. Moat bacon++! This code rocks; mt++. Shame that lazy bb-- didn't do it.");
}

fn go(s: &str) -> () {
    println!("{:?}", parse_many(s).unwrap().1);
}
