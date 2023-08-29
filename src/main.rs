use nom::{
    bytes::complete::tag,
    bytes::complete::{is_a, is_not},
    character::complete::alpha1,
    combinator::opt,
    multi::fold_many1,
    sequence::terminated,
    IResult,
};
use std::collections::HashMap;

fn parse_many(i: &str) -> IResult<&str, HashMap<&str, i32>> {
    let letters = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcedfghijklmnopqrstuvwxyz+-";
    let words = terminated(is_a(letters), opt(is_not(letters)));
    let mut upvote = opt(terminated(
        alpha1::<&str, nom::error::Error<&str>>,
        tag("++"),
    ));
    let mut downvote = opt(terminated(
        alpha1::<&str, nom::error::Error<&str>>,
        tag("--"),
    ));
    fold_many1(words, HashMap::new, move |mut acc: HashMap<_, _>, item| {
        if let Ok((_, Some(term))) = upvote(item) {
            let count = acc.entry(term).or_insert(0);
            *count += 1;
        } else if let Ok((_, Some(term))) = downvote(item) {
            let count = acc.entry(term).or_insert(0);
            *count -= 1;
        }
        acc
    })(i)
}

fn main() {
    println!("{:?}", parse_many("bacon++"));
    println!("{:?}", parse_many("bacon++. Oh dear emacs crashed"));
    println!(
        "{:?}",
        parse_many("Drivel about LISP. bacon++. Oh dear emacs crashed")
    );
    println!(
        "{:?}",
        parse_many("Drivel about LISP. bacon++. Oh dear emacs crashed. Moat bacon++! This code rocks; mt++. Shame that lazy bb-- didn't do it.")
    );
}
