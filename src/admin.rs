pub mod admin_proto {
    tonic::include_proto!("admin.v1");
}

use admin_proto::{
    karma_service_server::{KarmaService, KarmaServiceServer},
    plugins_service_server::{PluginsService, PluginsServiceServer},
    ListRequest, ListResponse, PluginInfo, SetRequest, SetResponse,
};
use thiserror::Error;
use tokio_graceful_shutdown::SubsystemHandle;
use tonic::{transport::Server, Request, Response, Status};
use tracing::*;

use crate::{karma::Karma, plugins::WasmPlugins};

#[derive(Error, Debug)]
pub enum AdminError {
    #[error("Bind error")]
    Bind(#[from] std::net::AddrParseError),
    #[error("Runtime error")]
    Runtime(#[from] tonic::transport::Error),
}

pub struct Admin {
    k: Karma,
    ps: WasmPlugins,
}
impl Admin {
    pub fn new(k: Karma, ps: WasmPlugins) -> Self {
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
    async fn set(&self, request: Request<SetRequest>) -> Result<Response<SetResponse>, Status> {
        debug!(?request, "Got karma set request");

        let old = self.k.set(&request.get_ref().term, request.get_ref().value);

        let reply = SetResponse { old_value: old };

        Ok(Response::new(reply))
    }
}

struct PluginsSrv {
    ps: WasmPlugins,
}
impl PluginsSrv {
    fn new(ps: WasmPlugins) -> Self {
        Self { ps }
    }
}
#[tonic::async_trait]
impl PluginsService for PluginsSrv {
    async fn list(&self, request: Request<ListRequest>) -> Result<Response<ListResponse>, Status> {
        debug!(?request, "Got plugins list request");

        self.ps.ps.iter().for_each(|p| info!(?p.path, p.size, "Plugin"));

        Ok(Response::new(ListResponse {
            plugins: self
                .ps
                .ps
                .iter()
                .map(|p| PluginInfo {
                    // TODO: make optional in the proto
                    name: p.path.file_prefix().map(|p| p.to_string_lossy().to_string()).unwrap_or("<unknown>".to_owned()),
                    path: p.path.to_string_lossy().to_string(),
                    size: p.size.unwrap_or(0),
                })
                .collect(),
        }))
    }
}
