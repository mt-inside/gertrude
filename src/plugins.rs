use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock},
    time::{Duration, SystemTime},
};

use notify_debouncer_full::{
    new_debouncer,
    notify::{EventKind, RecursiveMode, Watcher},
    DebounceEventResult,
};
use thiserror::Error;
use tokio_graceful_shutdown::SubsystemHandle;
use tracing::*;
use wasmer::{imports, Module, Store};

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

#[derive(Clone)]
pub struct WasmPlugins {
    dir: Option<String>,
    pub ps: Arc<RwLock<Vec<WasmPlugin>>>,
}

impl WasmPlugins {
    // TODO: use dyn Box<Trait> over two types for empty/non-empty
    pub fn new(plugin_dir: Option<&str>) -> Self {
        let mut ps = vec![];

        if let Some(plugin_dir) = plugin_dir {
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
        }

        Self {
            dir: plugin_dir.map(str::to_owned),
            ps: Arc::new(RwLock::new(ps)),
        }
    }

    pub async fn watch(self, subsys: SubsystemHandle) -> Result<(), anyhow::Error> {
        if let Some(ref dir) = self.dir {
            match Path::new(dir).canonicalize() {
                Ok(ref plugin_dir) => {
                    info!(?plugin_dir, "Plugin manager watching");

                    let this = self.clone(); // give closure its own copy, cause it runs on a background thread so can't reason about lifetimes

                    let mut debouncer = new_debouncer(Duration::from_secs(2), None, move |result: DebounceEventResult| match result {
                        Err(errors) => errors.iter().for_each(|error| error!(?error, "directory watch")),
                        Ok(events) => {
                            debug!(?events, "directory watch");
                            // TODO: handle deletes etc
                            // TODO: reload when the file changes
                            this.ps.write().unwrap().extend(
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
        }

        // debouncer stops on drop (kills its bg thread) on drop
        subsys.on_shutdown_requested().await;
        info!("Plugins manager task got shutdown request");

        Ok(())
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
