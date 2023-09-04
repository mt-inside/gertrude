use actix_web::{get, HttpRequest, HttpResponse, Responder};
use prometheus::{register_counter_vec_with_registry, register_gauge_vec_with_registry, CounterVec, Encoder, GaugeVec, Registry, TextEncoder};

use super::http_srv::SrvData;

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

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

#[get("/metrics")]
async fn handle_metrics(data: actix_web::web::Data<SrvData>, _req: HttpRequest) -> impl Responder {
    let encoder = TextEncoder::new();
    let mut buffer = vec![];
    encoder.encode(&data.metrics.reg.gather(), &mut buffer).unwrap();
    HttpResponse::Ok().body(buffer)
}
