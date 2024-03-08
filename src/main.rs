use actix_web::{web, App, HttpServer};
use awc::Client;
mod mw;
mod reverse_proxy;

pub struct AppData {
    pub client: Client,
}
impl AppData {
    fn new(client: Client) -> Self {
        AppData { client }
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    HttpServer::new(|| {
        App::new()
            .wrap(actix_web::middleware::Logger::default())
            .wrap(mw::limiter::RequestLimiter::new(5))
            .wrap(mw::auth_parse::CheckAuth)
            .app_data(web::Data::new(AppData::new(Client::default())))
            .default_service(web::route().to(reverse_proxy::forward))
    })
    .bind("127.0.0.1:9080")?
    .run()
    .await
}
