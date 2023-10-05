/* Karma
 * Case-senitivity of map is a business decision, hence all these methods take
 * a) strings, rather than say UniCase
 * b) Vec<tuple>, as HashMaps require decisions on key equality.
 * Hence, UniCase is considered a business decision isolated to this file, thus that type doesn't leak from here
// TODO: terms will take on the first case seen. Would be much easier to just to_lower() user input... (keep it in this file though, as that's a business decision for karma tracking)
 */

pub mod storage_proto {
    tonic::include_proto!("storage.v1");
}

use std::{
    collections::HashMap,
    fmt,
    fs::File,
    io::{Read, Seek, Write},
    path::PathBuf,
    sync::{Arc, Mutex, RwLock},
};

use prost::Message;
use storage_proto::Karmae;
use tracing::*;
use unicase::UniCase;

use crate::metrics::Metrics;

// TODO this type should persist to disk on updates, and read from disk when constructed.
// - just serialize to protos
#[derive(Clone, Debug)]
pub struct Karma {
    k: Arc<RwLock<HashMap<UniCase<String>, i32>>>,
    persist_file: Option<Arc<Mutex<File>>>,
    metrics: Metrics,
}

impl Karma {
    // TODO Should take non-uni, &str
    fn new_inner(k: HashMap<String, i32>, persist_file: Option<File>, metrics: Metrics) -> Self {
        let ret = Self {
            k: Arc::new(RwLock::new(k.into_iter().map(|(k, v)| (UniCase::new(k), v)).collect())),
            persist_file: persist_file.map(|f| Arc::new(Mutex::new(f))),
            metrics,
        };
        ret.update_metrics();
        ret
    }

    // TODO: impl default, with an empty metricsa. cfg test only?
    #[allow(dead_code)]
    pub fn new(metrics: Metrics) -> Self {
        Self::new_inner(HashMap::new(), None, metrics)
    }

    pub fn from_file(file_name: Option<&str>, metrics: Metrics) -> Self {
        // TODO: effectful layer? This mod should just deal in byte bufs in and out; some layer/composed object should deal with file I/O
        // TODO: don't hold the file open. Feels unix-y, but it's not a log file. User wants to have it write once, mv the file, and have it write a new one. Will simplify a lot of this logic (remove mutex from persist_path)
        // - you also don't actually wanna canonicalise either - let the user work out what the error is. Certainly don't canon and store, cause again the user might wanna insert a symlink in the path at runtime

        fn read_karmae(f: &mut File) -> anyhow::Result<Karmae> {
            let mut buf = Vec::with_capacity(f.metadata()?.len() as usize);
            f.read_to_end(&mut buf)?;
            let k = Karmae::decode(&buf[..])?;
            Ok(k)
        }

        // It's fine for this fn to deal with None file_name, as in future eg we might wanna start falling back to ~
        if let Some(file_name) = file_name {
            let path = PathBuf::from(file_name);
            match path.try_exists() {
                Ok(exists) => {
                    let res = if exists {
                        File::options().read(true).write(true).open(&path)
                    } else {
                        info!(?path, "File didn't exist; creating");
                        File::options().read(true).write(true).create_new(true).open(&path)
                    };

                    match res {
                        Ok(mut file) => match read_karmae(&mut file) {
                            Ok(k) => {
                                info!(?path, "Loaded from file");
                                Self::new_inner(k.values, Some(file), metrics)
                            }
                            Err(e) => {
                                error!(?e, ?path, "Can't deserialize karma from file. Won't persist.");
                                Self::new_inner(HashMap::new(), None, metrics)
                            }
                        },
                        Err(e) => {
                            error!(?e, ?path, "Can't open for read & write. Won't persist.");
                            Self::new_inner(HashMap::new(), None, metrics)
                        }
                    }
                }
                Err(e) => {
                    error!(?e, ?path, "Can't determine if file exists. Won't persist.");
                    Self::new_inner(HashMap::new(), None, metrics)
                }
            }
        } else {
            info!("No path given, not persisting");
            Self::new_inner(HashMap::new(), None, metrics)
        }
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

        self.update_metrics();
        let _res = self.persist();

        old
    }

    // Convenience method to do a bulk update from a set of differences
    pub fn bias_from(&self, biases: Vec<(&str, i32)>) {
        let mut write = self.k.write().unwrap();
        // TODO factor this out. Can't be a from or anything. But have From use it.
        biases.into_iter().map(move |(k, v)| (UniCase::new(k.to_owned()), v)).for_each(|(k, v)| {
            let cur = write.entry(k).or_insert(0);
            *cur += v;
        });
        drop(write);

        self.update_metrics();
        let _res = self.persist();
    }

    fn update_metrics(&self) {
        info!(%self, "Karma");

        // Since prom is a polling system, all we're doing here is setting metrics locally, which
        // is cheap. Hence we just "publish" them all every time, rather than trying to update the
        // minimal set.
        self.k.read().unwrap().iter().for_each(|(k, v)| self.metrics.karma.with_label_values(&[k]).set(*v as f64));
    }

    fn persist(&self) -> anyhow::Result<()> {
        if let Some(ref file) = self.persist_file {
            let read = self.k.read().unwrap();
            let mut write = file.lock().unwrap();

            let ks = Karmae {
                // TODO make this an into (Karma into karmae)
                values: (*read).iter().map(|(k, v)| (k.to_owned().to_string(), *v)).collect(),
            };

            let mut buf = vec![];
            ks.encode(&mut buf)?;
            info!(len = buf.len(), "Buf");

            info!("Persisting");
            (*write).set_len(0)?;
            (*write).rewind()?;
            (*write).write_all(&buf)?;
        } else {
            trace!("Not persisting");
        }

        Ok(())
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
            persist_file: None,
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
