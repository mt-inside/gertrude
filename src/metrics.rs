use actix_web::{dev::Server, get, middleware, web::Data, App, HttpRequest, HttpResponse, HttpServer, Responder};
use maplit::hashmap;
use prometheus::{register_gauge_vec_with_registry, Encoder, GaugeVec, Registry, TextEncoder};
use tokio_graceful_shutdown::SubsystemHandle;
use tracing::*;

#[derive(Clone)]
pub struct Metrics {
    reg: Registry,
    pub karma: GaugeVec,
}

impl Metrics {
    pub fn new() -> Self {
        let reg = Registry::new();
        let karma = register_gauge_vec_with_registry!("karma", "vox populi", &["term"], reg).unwrap();
        Metrics { reg, karma }
    }

    fn server(self) -> Server {
        let r = self.reg.clone();

        HttpServer::new(move || {
            App::new()
                .app_data(Data::new(r.clone()))
                .wrap(middleware::Logger::default().exclude("/health"))
                .service(health)
                .service(metrics)
        })
        .disable_signals()
        .bind("0.0.0.0:8888")
        .expect("Can't bind to ::8888")
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

// TODO: move me (and the rest of the http serving gubbins. Metrics shouldn't own serve, rather serve should take a metrics handler - at which point Data will need to be everything all the handlers need; a custom struct, not just a Registry)
#[get("/healthz")]
async fn health(_data: actix_web::web::Data<Registry>, _req: HttpRequest) -> impl Responder {
    HttpResponse::Ok().json(hashmap!["health"=> "ok", "name" => crate::NAME, "version" => crate::VERSION])
}

#[get("/metrics")]
async fn metrics(data: actix_web::web::Data<Registry>, _req: HttpRequest) -> impl Responder {
    let encoder = TextEncoder::new();
    let mut buffer = vec![];
    encoder.encode(&data.gather(), &mut buffer).unwrap();
    HttpResponse::Ok().body(buffer)
}
