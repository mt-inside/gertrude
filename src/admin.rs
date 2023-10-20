pub mod admin_proto {
    tonic::include_proto!("admin.v1");
}

use admin_proto::{
    karma_service_server::{KarmaService, KarmaServiceServer},
    plugins_service_server::{PluginsService, PluginsServiceServer},
    KarmaSetRequest, KarmaSetResponse, PluginInfo, PluginsListRequest, PluginsListResponse,
};
use thiserror::Error;
use tokio_graceful_shutdown::SubsystemHandle;
use tonic::{transport::Server, Request, Response, Status};
use tracing::*;

use crate::{karma::Karma, plugins::Foo};

#[derive(Error, Debug)]
pub enum AdminError {
    #[error("Bind error")]
    Bind(#[from] std::net::AddrParseError),
    #[error("Runtime error")]
    Runtime(#[from] tonic::transport::Error),
}

pub struct Admin {
    k: Karma,
    ps: Foo,
}
impl Admin {
    pub fn new(k: Karma, ps: Foo) -> Self {
        Self { k, ps }
    }

    pub async fn serve(self, subsys: SubsystemHandle) -> Result<(), AdminError> {
        let addr = "[::1]:50051".parse()?;

        info!(%addr, "Serving admin gRPC interface");

        Server::builder()
            .add_service(KarmaServiceServer::new(KarmaSrv::new(self.k)))
            .add_service(PluginsServiceServer::new(PluginsSrv::new(self.ps)))
            // Tonic wants notifying of shutdown by a future that completes
            .serve_with_shutdown(addr, async {
                subsys.on_shutdown_requested().await;
                info!("gRPC server task got shutdown request");
            })
            .await
            .map_err(AdminError::Runtime)
    }
}

struct KarmaSrv {
    k: Karma,
}
impl KarmaSrv {
    fn new(k: Karma) -> Self {
        Self { k }
    }
}
#[tonic::async_trait]
impl KarmaService for KarmaSrv {
    async fn set(&self, request: Request<KarmaSetRequest>) -> Result<Response<KarmaSetResponse>, Status> {
        info!(?request, "Got karma set request");

        let old = self.k.set(&request.get_ref().term, request.get_ref().value);

        let reply = KarmaSetResponse { old_value: old };

        Ok(Response::new(reply))
    }
}

struct PluginsSrv {
    ps_mgr: Foo,
}
impl PluginsSrv {
    fn new(ps_mgr: Foo) -> Self {
        Self { ps_mgr }
    }
}
#[tonic::async_trait]
impl PluginsService for PluginsSrv {
    async fn list(&self, request: Request<PluginsListRequest>) -> Result<Response<PluginsListResponse>, Status> {
        info!(?request, "Got plugins list request");

        Ok(Response::new(PluginsListResponse {
            plugins: self
                .ps_mgr
                .get_info()
                .iter()
                // TODO: make this an Into / From (whichever would go in this file cause PluginInfo is our type)
                .map(|p| PluginInfo {
                    // TODO: plugins should have to impliment a name function (no, see below), to avoid this nastyness. Can then also print it when they're loaded. Version too.
                    // - Is this what wasmpack does? YES. builds to wasm, makes JS wrapper files, makes npm package.json. Load plugins this: decompress, read package.json (expect to be npm-compat), filter files list to *.wasm, assert only 1, load. Use name etc from package.json.
                    // Can make these with wasm-pack: need to set metadata in cargo.toml, then call wasm-pack pack
                    name: p.path.file_prefix().map(|p| p.to_string_lossy().to_string()).unwrap_or("<unknown>".to_owned()),
                    path: p.path.to_string_lossy().to_string(),
                    size: p.size.unwrap_or(0),
                    loaded_time: Some(p.loaded_time.into()),
                })
                .collect(),
        }))
    }
}
