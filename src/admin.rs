pub mod admin_proto {
    tonic::include_proto!("admin.v1");
}

use admin_proto::{
    karma_service_server::{KarmaService, KarmaServiceServer},
    SetRequest, SetResponse,
};
use thiserror::Error;
use tokio_graceful_shutdown::SubsystemHandle;
use tonic::{transport::Server, Request, Response, Status};
use tracing::*;

use crate::karma::Karma;

#[derive(Error, Debug)]
pub enum AdminError {
    #[error("Bind error")]
    Bind(#[from] std::net::AddrParseError),
    #[error("Runtime error")]
    Runtime(#[from] tonic::transport::Error),
}

pub struct Admin {
    k: Karma,
}

impl Admin {
    pub fn new(k: Karma) -> Self {
        Self { k }
    }

    pub async fn serve(self, subsys: SubsystemHandle) -> Result<(), AdminError> {
        let addr = "[::1]:50051".parse()?;

        info!(%addr, "Serving admin gRPC interface");

        Server::builder()
            .add_service(KarmaServiceServer::new(self))
            // Tonic wants notifying of shutdown by a future that completes
            .serve_with_shutdown(addr, async {
                subsys.on_shutdown_requested().await;
                info!("gRPC server task got shutdown request");
            })
            .await
            .map_err(AdminError::Runtime)
    }
}

#[tonic::async_trait]
impl KarmaService for Admin {
    async fn set(&self, request: Request<SetRequest>) -> Result<Response<SetResponse>, Status> {
        debug!(?request, "Got karma set request");

        let old = self.k.set(&request.get_ref().term, request.get_ref().value);

        let reply = SetResponse { old_value: old };

        Ok(Response::new(reply))
    }
}
