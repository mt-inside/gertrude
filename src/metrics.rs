use actix_web::{dev::Server, get, middleware, web::Data, App, HttpRequest, HttpResponse, HttpServer, Responder};
use maplit::hashmap;
use prometheus::{register_counter_vec_with_registry, register_gauge_vec_with_registry, CounterVec, Encoder, GaugeVec, Registry, TextEncoder};
use tokio_graceful_shutdown::SubsystemHandle;
use tracing::*;

#[derive(Clone)]
pub struct Metrics {
    reg: Registry,
    pub karma: GaugeVec,
    pub messages: CounterVec,
    pub dms: CounterVec,
    pub pings: CounterVec,
    pub pongs: CounterVec,
}

impl Metrics {
    pub fn new() -> Self {
        let reg = Registry::new();
        let karma = register_gauge_vec_with_registry!("karma", "vox populi", &["term"], reg).unwrap();
        let messages = register_counter_vec_with_registry!("messages", "all messages", &["command"], reg).unwrap();
        let dms = register_counter_vec_with_registry!("dms", "messages directed at us", &["from", "respondee"], reg).unwrap();
        let pings = register_counter_vec_with_registry!("pings", "from server", &["server"], reg).unwrap();
        let pongs = register_counter_vec_with_registry!("pongs", "to server", &["server"], reg).unwrap();
        Metrics {
            reg,
            karma,
            messages,
            dms,
            pings,
            pongs,
        }
    }
}

pub struct HTTPSrv {
    addr: String,
    m: Metrics,
}

// TODO: need a proper Data type, rather than it just _being_ Registry

impl HTTPSrv {
    pub fn new(addr: String, m: Metrics) -> Self {
        Self { addr, m }
    }

    fn server(self) -> Server {
        let r = self.m.reg.clone();

        HttpServer::new(move || {
            App::new()
                .app_data(Data::new(r.clone()))
                .wrap(middleware::Logger::default().exclude("/healthz"))
                .service(health)
                .service(metrics)
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

// TODO: move me (and the rest of the http serving gubbins)
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
