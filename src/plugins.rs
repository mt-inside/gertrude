use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock, RwLockReadGuard},
    time::{Duration, SystemTime},
};

use notify_debouncer_full::{
    new_debouncer,
    notify::{EventKind, RecursiveMode, Watcher as NotifyWatcher},
    DebounceEventResult,
};
use thiserror::Error;
use tokio_graceful_shutdown::SubsystemHandle;
use tracing::*;
use wasmer::{imports, Module, Store};

trait Watcher {
    fn initial_load(&self);
    fn watch(self);
}

// struct Watchers {
//     fs: FsWatcher,
// }

// don't need the syn on the reader, just the watcher
// new should prlly return Option<watcher> and None for nothing to watch
// note: we might have >1 source, so ideally new_manager would spawn all the tasks for all the sources
// - return joinset of watchers? Probably uber-task over them. No dyn, just a manager-manager
// - watcher-watcher: either one Opt for each type, but you might want FsWatch on >1 dir, so vec of
// dyn Watcher - no just join handles
// - trait for watchers (with initial() and watch()) but we don't store them dyn, just join handles
// - builder pattern keeps vecs of paths to watch, urls to poll ,etc
pub fn new_plugins(plugin_dir: Option<&str>) -> (FsWatcher, Manager) {
    let mut ps = vec![];

    let plugin_dir = plugin_dir.unwrap(); // FIXME

    // Test me!
    //     then:
    // ctor every plugin type based on args, push to vec<dyn Watcher>
    // pass ps to foo.init() one at a time
    // call foo.watch() one at a time, save join handles

    //if let Some(plugin_dir) = plugin_dir {
    // TODO: canon path here
    // notify doesn't seem to have a mode where it emits Create events for existing files, so we read the dir here.
    info!(plugin_dir, "Loading initial plugins");
    ps.extend(match fs::read_dir(plugin_dir) {
        Ok(entries) => entries.filter_map(|e| e.ok()).filter_map(|dent| WasmPlugin::new(&dent.path())).collect(),
        Err(e) => {
            error!(?e, "Can't read plugin dir");
            vec![]
        }
    });

    let ps_wrapped = Arc::new(RwLock::new(ps));

    let fs_watcher = FsWatcher {
        dir: plugin_dir.to_owned(),
        ps: ps_wrapped.clone(),
    };

    (fs_watcher, Manager { ps: ps_wrapped.clone() })
}

pub struct FsWatcher {
    dir: String,
    ps: Arc<RwLock<Vec<WasmPlugin>>>,
}

impl FsWatcher {
    pub async fn watch(self, _subsys: SubsystemHandle) -> Result<(), anyhow::Error> {
        let ps = self.ps.clone(); // give closure its own copy, cause it runs on a background thread so can't reason about lifetimes

        match Path::new(&self.dir).canonicalize() {
            Ok(ref plugin_dir) => {
                info!(?plugin_dir, "Plugin manager watching");

                let mut debouncer = new_debouncer(Duration::from_secs(2), None, move |result: DebounceEventResult| match result {
                    Err(errors) => errors.iter().for_each(|error| error!(?error, "directory watch")),
                    Ok(events) => {
                        debug!(?events, "directory watch");
                        // TODO: handle deletes etc
                        // TODO: reload when the file changes
                        ps.write().unwrap().extend(
                            events
                                .into_iter()
                                .filter(|e| e.kind == EventKind::Create(notify_debouncer_full::notify::event::CreateKind::File))
                                .flat_map(move |event| event.paths.clone().into_iter().filter_map(|path| WasmPlugin::new(&path))),
                        );
                    }
                })
                .unwrap();

                debouncer.watcher().watch(plugin_dir, RecursiveMode::NonRecursive).unwrap();
                debouncer.cache().add_root(plugin_dir, RecursiveMode::NonRecursive);
            }
            Err(e) => error!(?e, "Can't watch plugin dir"),
        }

        // debouncer stops on drop (kills its bg thread) on drop. TODO how is this working, cause it's dropped by now?
        //subsys.on_shutdown_requested().await;
        info!("Plugins manager task got shutdown request");

        Ok(())
    }
}

#[derive(Clone)]
pub struct Manager {
    // TODO: hideous this has to be here. Can't construct a fake one just in get_info cause you're
    // returning a borrow of a local. I'm actually surprised the lifetimes work to do this, but
    // maybe it's all the Arcs. What's a nicer solution? Do try returning borrows of the Vec items
    // in FsWatcher, but I don't think it'll work
    // - ofc the real soltuion is the pluginInfo type, and can just return an empty vec of it from get_info()
    ps: Arc<RwLock<Vec<WasmPlugin>>>,
}

impl Manager {
    // TODO: best practice is not to return these guards. Otoh we don't really wanna clone the contents.
    // Stop being lazy and build an Info type, so we don't have to clone the actual plugin & store
    // I _think_ this is the idiomatic thing to return.
    // - Can't return borrows to the Vec items, cause we're going across threads so lifetimes
    // - Don't wanna clone it, cause it's got a plugin struct and a store
    //   - Unless we Arc the whole thing? Feels bad - don't want the viewer to be able to hold the
    //   thing alive when we've tried to unload it. Would have to represent the unloaded state for
    //   viewers. Want the type to say "quickly grab what you want; don't hold these objects around
    //   and keep observing them"
    // - LockGuard is horrible though cause we need to allocate a lock of an empty vec for noplugins
    pub fn get_info(&self) -> RwLockReadGuard<Vec<WasmPlugin>> {
        self.ps.read().unwrap()
    }

    // This class and method iterate the plugins because we might want to do fancy stuff like run them in parallel
    pub fn handle_privmsg(&self, msgs: &[&str]) -> Vec<String> {
        // TODO: when they do network i/o, run in parallel
        self.ps
            .read()
            .unwrap()
            .iter()
            .filter_map(|p| {
                debug!("Calling plugin TODO");
                match p.p.handle_privmsg(&mut p.store.lock().unwrap(), msgs) {
                    Ok(reply) => reply,
                    Err(e) => {
                        warn!(?e, "Plugin TODO error");
                        None
                    }
                }
            })
            .collect()
    }
}

/* wtf?
 * - interface defined in wai file. Can include non-WASM types like strings and structs (records)
 * - using the macro crates in https://github.com/wasmerio/wai to generate bindings on both sides
 *   - rust::export in the plugins (as they export the interface)
 *   - wasmer::import here (as we're importing the interface into a wasmer engine)
 *   - these use the same underlying bindgen code, which also has a cli: https://github.com/wasmerio/wai (though I can't make it work)
 *   - use `cargo expand` to see what those macros expand to
 * - this is a fork of https://github.com/bytecodealliance/wit-bindgen, who refused to add wasmer as a target
 */
wai_bindgen_wasmer::import!("idl/internal/wasm/plugin.wai");

#[derive(Error, Debug)]
pub enum WasmError {
    #[error("Load error")]
    Load(#[from] std::io::Error),
    #[error("Compile error")]
    Compile(#[from] wasmer::IoCompileError),
    #[error("Instantiation error")]
    Instantiate(#[from] anyhow::Error),
    #[error("Runtime error")]
    Runtime(#[from] wasmer::RuntimeError),
}

pub struct WasmPlugin {
    p: plugin::Plugin,
    store: Mutex<Store>,
    pub path: PathBuf,
    pub size: Option<u64>,
    pub loaded_time: SystemTime,
}

impl WasmPlugin {
    fn new(path: &PathBuf) -> Option<WasmPlugin> {
        // TODO attempt to canonicalize path here. Will a) canon it, b) flush out non-existant etc
        if let Some(os_ext) = path.extension() {
            if let Some(ext) = os_ext.to_str() {
                if ext.to_lowercase() == "wasm" {
                    info!(?path, "loading plugin");

                    let mut store = Store::default();
                    match Module::from_file(&store, path) {
                        Ok(module) => {
                            // TODO: give them a host fn that takes URI and headers map, and gives string? json?
                            let mut imports = imports! {};
                            match plugin::Plugin::instantiate(&mut store, &module, &mut imports) {
                                Ok((p, _instance)) => {
                                    return Some(WasmPlugin {
                                        p,
                                        path: path.clone(),
                                        size: fs::metadata(path).map(|m| m.len()).ok(),
                                        store: Mutex::new(store),
                                        loaded_time: SystemTime::now(),
                                    });
                                }
                                Err(e) => {
                                    error!(?e, "Failed to instantiate WASM plugin");
                                }
                            }
                        }
                        Err(e) => {
                            error!(?e, "Failed to load WASM plugin");
                        }
                    }
                }
            }
        }
        None
    }
}
