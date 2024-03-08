use actix_web::{middleware, web, App, HttpServer};
use awc::Client;
mod limiter;
mod md;
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
            .wrap(middleware::Logger::default())
            .wrap(md::CheckAuth)
            .app_data(web::Data::new(AppData::new(Client::default())))
            .default_service(web::route().to(reverse_proxy::forward))
    })
    .bind("127.0.0.1:9080")?
    .run()
    .await
}
