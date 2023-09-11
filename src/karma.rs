/* Karma
 * Case-senitivity of map is a business decision, hence all these methods take
 * a) strings, rather than say UniCase
 * b) Vec<tuple>, as HashMaps require decisions on key equality.
 * Hence, UniCase is considered a business decision isolated to this file, thus that type doesn't leak from here
// TODO: terms will take on the first case seen. Would be much easier to just to_lower() user input... (keep it in this file though, as that's a business decision for karma tracking)
 */
use std::{
    collections::HashMap,
    fmt,
    sync::{Arc, RwLock},
};

use tracing::*;
use unicase::UniCase;

use crate::metrics::Metrics;

// TODO this type should persist to disk on updates, and read from disk when constructed.
// - just serialize to protos
#[derive(Clone, Debug)]
pub struct Karma {
    k: Arc<RwLock<HashMap<UniCase<String>, i32>>>,
    metrics: Metrics,
}

impl Karma {
    pub fn new(metrics: Metrics) -> Self {
        Self {
            k: Arc::new(RwLock::new(HashMap::new())),
            metrics,
        }
    }

    pub fn get(&self, term: &str) -> i32 {
        let read = self.k.read().unwrap();
        let val = read.get(&UniCase::new(term.to_owned())).unwrap_or(&0);
        *val
    }

    pub fn set(&self, term: &str, new: i32) -> i32 {
        let term = UniCase::new(term.to_owned());
        let mut write = self.k.write().unwrap();
        let cur = write.entry(term.clone()).or_insert(0);
        let old = *cur;
        *cur = new;
        drop(write);

        self.publish(&term, &new);

        old
    }

    pub fn bias_from(&self, biases: Vec<(&str, i32)>) {
        let mut write = self.k.write().unwrap();
        let bs = biases.into_iter().map(move |(k, v)| (UniCase::new(k.to_owned()), v)).fold(HashMap::new(), |mut acc, (k, v)| {
            let cur = acc.entry(k).or_insert(0);
            *cur += v;
            acc
        });
        for (k, v) in &bs {
            self.publish(k, v);
        }
        write.extend(bs);
    }

    fn publish(&self, term: &UniCase<String>, val: &i32) {
        info!(%self, "Karma");

        self.metrics.karma.with_label_values(&[&term]).set(*val as f64);
    }
}

impl fmt::Display for Karma {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let m = self.k.read().unwrap();
        let mut v: Vec<(&UniCase<String>, &i32)> = m.iter().collect();
        v.sort_by(|a, b| b.1.cmp(a.1));
        let mut first = true;
        let render = v.iter().fold(String::new(), |mut acc, (k, v)| {
            if first {
                first = false
            } else {
                acc.push_str("; ")
            }
            acc.push_str(&format!("{}: {}", k, v));
            acc
        });
        write!(f, "{}", render)
    }
}

#[cfg(test)]
impl From<HashMap<&str, i32>> for Karma {
    fn from(m: HashMap<&str, i32>) -> Self {
        Self {
            k: Arc::new(RwLock::new(m.into_iter().map(move |(k, v)| (UniCase::new(k.to_owned()), v)).collect())),
            metrics: Default::default(),
        }
    }
}

#[cfg(test)]
impl PartialEq<HashMap<&str, i32>> for Karma {
    // Checks that the set of keys is identical, and that their values match
    fn eq(&self, m: &HashMap<&str, i32>) -> bool {
        let k = self.k.read().expect("deadlock");

        let k_covers_m = m.iter().fold(true, |acc, (k1, v1)| {
            acc && match k.get(&UniCase::new(k1.to_owned().to_owned())) {
                None => false,
                Some(v2) => v1 == v2,
            }
        });
        let m_covers_k = k.iter().fold(true, |acc, (k1, _)| acc && m.get(k1.as_str()).is_some());

        k_covers_m && m_covers_k
    }
}

#[cfg(test)]
mod tests {
    use maplit::hashmap;

    use super::*;

    #[test]
    fn test_display() {
        let k = Karma::from(hashmap![
            "bacon" => 1,
            "blɸwback" => -1,
            "rust" => 666,
            "LISP" => -666,
        ]);
        assert_eq!(format!("{}", k), "rust: 666; bacon: 1; blɸwback: -1; LISP: -666");
    }
}
