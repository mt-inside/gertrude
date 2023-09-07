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
wai_bindgen_wasmer::import!("api/wasm/plugin.wai");

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
    dir: String,
    // TODO: can remove inner Arc?
    pub ps: Arc<RwLock<Vec<Arc<WasmPlugin>>>>,
}

impl WasmPlugins {
    // TODO: watch this directory for new files
    pub fn new(plugin_dir: &str) -> Self {
        // notify doesn't seem to have a mode where it emits Create events for existing files, so we read the dir here.
        info!(plugin_dir, "Loading initial plugins");
        let ps = match fs::read_dir(plugin_dir) {
            Ok(entries) => entries.filter_map(|e| e.ok()).filter_map(|dent| WasmPlugin::new(&dent.path())).map(Arc::new).collect(),
            Err(e) => {
                error!(?e, "Can't read plugin dir");
                vec![]
            }
        };

        Self {
            dir: plugin_dir.to_owned(),
            ps: Arc::new(RwLock::new(ps)),
        }
    }

    pub async fn watch(self, subsys: SubsystemHandle) -> Result<(), anyhow::Error> {
        let plugin_dir = Path::new(&self.dir).canonicalize().unwrap();
        info!(?plugin_dir, "Plugin manager watching");
        let this = self.clone(); // give closure its own copy, cause it runs on a background thread so can't reason about lifetimes

        let mut debouncer = new_debouncer(Duration::from_secs(2), None, move |result: DebounceEventResult| match result {
            Err(errors) => errors.iter().for_each(|error| error!(?error, "directory watch")),
            Ok(events) => {
                debug!(?events, "directory watch");
                // TODO: handle deletes etc
                this.ps.write().unwrap().extend(
                    events
                        .into_iter()
                        .filter(|e| e.kind == EventKind::Create(notify_debouncer_full::notify::event::CreateKind::File))
                        .flat_map(move |event| event.paths.clone().into_iter().filter_map(|path| WasmPlugin::new(&path)).map(Arc::new)),
                );
            }
        })
        .unwrap();
        debouncer.watcher().watch(&plugin_dir, RecursiveMode::NonRecursive).unwrap();
        debouncer.cache().add_root(plugin_dir, RecursiveMode::NonRecursive);

        // debouncer stops on drop (kills its bg thread) on drop
        subsys.on_shutdown_requested().await;
        info!("Bot task got shutdown request");

        Ok(())
    }

    pub fn handle_privmsg(&self, msg: &str) -> Vec<Result<String, WasmError>> {
        self.ps
            .read()
            .unwrap()
            .iter()
            .map(|p| p.p.handle_privmsg(&mut p.store.lock().unwrap(), msg).map_err(WasmError::Runtime))
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
        if let Some(os_ext) = path.extension() {
            if let Some(ext) = os_ext.to_str() {
                if ext.to_lowercase() == "wasm" {
                    info!(?path, "loading plugin");

                    let mut store = Store::default();
                    match Module::from_file(&store, path) {
                        Ok(module) => {
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
