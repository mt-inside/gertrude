use std::time::SystemTime;

use actix_web::{dev::Server, get, middleware, App, HttpRequest, HttpResponse, HttpServer, Responder};
use maplit::hashmap;
use tokio_graceful_shutdown::SubsystemHandle;
use tracing::*;

use super::metrics::{handle_metrics, Metrics};

#[derive(Clone)]
pub struct SrvData {
    pub metrics: Metrics,
    start_time: SystemTime,
}

pub struct HTTPSrv {
    addr: String,
    data: SrvData,
}

impl HTTPSrv {
    pub fn new(addr: String, metrics: Metrics) -> Self {
        let data = SrvData {
            metrics: metrics.clone(),
            start_time: SystemTime::now(),
        };
        Self { addr, data }
    }

    fn server(self) -> Server {
        HttpServer::new(move || {
            App::new()
                .app_data(actix_web::web::Data::new(self.data.clone()))
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
async fn handle_health(data: actix_web::web::Data<SrvData>, _req: HttpRequest) -> impl Responder {
    let start_time_rendered = format!("{:?}", data.start_time);
    let uptime_rendered = format!("{:?}", SystemTime::now().duration_since(data.start_time));

    HttpResponse::Ok().json(hashmap![
        "health" => "ok",
        "name" => crate::NAME,
        "version" => crate::VERSION,
        "start_time" => &start_time_rendered, // TODO format this nicely.
        "uptime" => &uptime_rendered, // TODO: unwrap, format in human units.
    ])
}
