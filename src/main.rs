use std::{env, fs::File, io::Read, path::Path};

use actix_web::{web, App, HttpServer};
use awc::Client;
use domain::config::GLOBAL_CONFIG;
use dotenv::dotenv;
mod domain;
mod mw;
mod reverse_proxy;
use env_logger::Env;

pub struct AppData {
    pub client: Client,
    pub config: domain::config::Config,
}
impl AppData {
    fn new(client: Client, config: domain::config::Config) -> Self {
        AppData { client, config }
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    dotenv().ok();

    // init logger with checking inside env_var wether timestamp is true or false
    env_logger::Builder::from_env(Env::default().default_filter_or("info"))
        .format_timestamp(None)
        .init();
    // Get the CONFIG_SETTING environment variable
    let config_setting = env::var("CONFIG_SETTING");

    let config: domain::config::Config;

    match config_setting {
        Ok(val) => {
            // If CONFIG_SETTING is set, parse it
            log::info!("Using config from environment variable");
            config = serde_json::from_str(&val).expect("JSON was not well-formatted");
        }
        Err(_) => {
            // If CONFIG_SETTING is not set, parse the file
            log::info!("Using config from config.json");
            let json_file_path = Path::new("config.json");
            let mut json_file = File::open(&json_file_path).expect("File open failed");
            let mut json_content = String::new();
            json_file
                .read_to_string(&mut json_content)
                .expect("File read failed");
            config = serde_json::from_str(&json_content).expect("JSON was not well-formatted");
        }
    }

    config.router();
    GLOBAL_CONFIG.set(config.clone()).unwrap();

    // get env port
    let port = env::var("PORT").unwrap_or_else(|_| config.port.to_string());

    HttpServer::new(move || {
        App::new()
            .wrap(actix_web::middleware::Logger::default())
            .wrap(mw::limiter::RequestLimiter::new(config.clone()))
            .wrap(mw::auth_parse::CheckAuth)
            .app_data(web::Data::new(AppData::new(
                Client::default(),
                config.clone(),
            )))
            .default_service(web::route().to(reverse_proxy::forward))
    })
    .bind(format!("0.0.0.0:{}", port))?
    .run()
    .await
}
