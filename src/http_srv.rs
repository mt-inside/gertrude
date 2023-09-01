use actix_web::{dev::Server, get, middleware, web::Data, App, HttpRequest, HttpResponse, HttpServer, Responder};
use maplit::hashmap;
use tokio_graceful_shutdown::SubsystemHandle;
use tracing::*;

use super::metrics::{handle_metrics, Metrics};

#[derive(Clone)]
pub struct SrvData {
    pub metrics: Metrics,
}

pub struct HTTPSrv {
    addr: String,
    data: SrvData,
}

impl HTTPSrv {
    pub fn new(addr: String, metrics: Metrics) -> Self {
        let data = SrvData { metrics: metrics.clone() };
        Self { addr, data }
    }

    fn server(self) -> Server {
        HttpServer::new(move || {
            App::new()
                .app_data(Data::new(self.data.clone()))
                .wrap(middleware::Logger::default().exclude("/healthz"))
                .service(handle_health)
                .service(handle_metrics)
        })
        .disable_signals()
        .bind(&self.addr)
        .unwrap_or_else(|_| panic!("Can't bind to {}", self.addr))
        .shutdown_timeout(5)
        .run()
    }

    pub async fn serve(self, subsys: SubsystemHandle) -> Result<(), std::io::Error> {
        let srv = self.server();
        let h = srv.handle();

        tokio::select! {
            _ = srv => {
                info!("actix HTTP server returned; requesting shutdown");
                subsys.request_shutdown();
            },
            _ = subsys.on_shutdown_requested() => {
                info!("HTTP server task got shutdown request");
                h.stop(true).await;
            },
        }

        Ok(())
    }
}

#[get("/healthz")]
async fn handle_health(_data: actix_web::web::Data<SrvData>, _req: HttpRequest) -> impl Responder {
    HttpResponse::Ok().json(hashmap!["health"=> "ok", "name" => crate::NAME, "version" => crate::VERSION])
}
