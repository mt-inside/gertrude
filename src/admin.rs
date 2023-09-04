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

use super::Karma;

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

    pub async fn serve(self, _subsys: SubsystemHandle) -> Result<(), AdminError> {
        let addr = "[::1]:50051".parse()?;

        info!(%addr, "Serving admin gRPC interface");

        // TODO: shutdown
        Server::builder().add_service(KarmaServiceServer::new(self)).serve(addr).await.map_err(|e| AdminError::Runtime(e))
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
