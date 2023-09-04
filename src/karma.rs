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
#[derive(Clone)]
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

    // TODO: actually impl From or Into (what's the diff?). Or impl Eq<HashMap<_>>
    #[cfg(test)]
    pub fn from_map(m: HashMap<UniCase<String>, i32>) -> Self {
        Self {
            k: Arc::new(RwLock::new(m)),
            metrics: Default::default(),
        }
    }

    #[cfg(test)]
    pub fn to_map(self) -> HashMap<UniCase<String>, i32> {
        Arc::try_unwrap(self.k).unwrap().into_inner().unwrap()
    }

    pub fn get(&self, term: &str) -> i32 {
        let read = self.k.read().unwrap();
        let val = read.get(&UniCase::new(term.to_owned())).unwrap_or(&0);
        *val
    }

    pub fn set(&self, term: &str, new: i32) -> i32 {
        let mut write = self.k.write().unwrap();
        let cur = write.entry(UniCase::new(term.to_owned())).or_insert(0);
        let old = *cur;
        *cur = new;
        drop(write);

        self.publish(term, new);

        old
    }

    pub fn bias(&self, term: &str, diff: i32) -> i32 {
        let mut write = self.k.write().unwrap();
        let cur = write.entry(UniCase::new(term.to_owned())).or_insert(0);
        *cur += diff;
        let new = *cur;
        drop(write);

        self.publish(term, new);

        new
    }

    fn publish(&self, term: &str, val: i32) {
        info!(%self, "Karma");

        self.metrics.karma.with_label_values(&[term]).set(val as f64);
    }
}

impl fmt::Display for Karma {
    // TODO: prettier, maybe sorted
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let read = self.k.read().unwrap();
        write!(f, "{:?}", read)
    }
}
