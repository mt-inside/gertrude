use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use thiserror::Error;
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

pub struct WasmPlugin {
    p: plugin::Plugin,
    store: Mutex<Store>,
    pub path: PathBuf,
    pub size: Option<u64>,
}
#[derive(Default, Clone)]
pub struct WasmPlugins {
    pub ps: Vec<Arc<WasmPlugin>>,
}

impl WasmPlugins {
    // TODO: watch this directory for new files
    pub fn new(plugin_dir: Option<&str>) -> Self {
        match plugin_dir {
            None => Default::default(),
            Some(plugin_dir) => Self {
                ps: match fs::read_dir(plugin_dir) {
                    Ok(entries) => entries.filter_map(try_load).map(Arc::new).collect(),
                    Err(e) => {
                        error!(?e, "can't read plugin dir");
                        Default::default()
                    }
                },
            },
        }
    }

    pub fn handle_privmsg(&self, msg: &str) -> Vec<Result<String, WasmError>> {
        self.ps.iter().map(|p| p.p.handle_privmsg(&mut p.store.lock().unwrap(), msg).map_err(WasmError::Runtime)).collect()
    }
}

// TODO: call me new()
fn try_load(entry: Result<fs::DirEntry, std::io::Error>) -> Option<WasmPlugin> {
    debug!(?entry, "plugin dir");
    if let Ok(dent) = entry {
        if let Some(os_ext) = dent.path().extension() {
            if let Some(ext) = os_ext.to_str() {
                if ext.to_lowercase() == "wasm" {
                    info!(?dent, "loading plugin");

                    let mut store = Store::default();
                    match Module::from_file(&store, dent.path()) {
                        Ok(module) => {
                            let mut imports = imports! {};
                            match plugin::Plugin::instantiate(&mut store, &module, &mut imports) {
                                Ok((p, _instance)) => {
                                    return Some(WasmPlugin {
                                        p,
                                        path: dent.path(),
                                        size: dent.metadata().map(|m| m.len()).ok(),
                                        store: Mutex::new(store),
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
    }
    None
}
