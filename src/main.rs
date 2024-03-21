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
    pub host: String,
}
impl AppData {
    fn new(client: Client, host: String) -> Self {
        AppData { client, host }
    }
}

// Use Jemalloc only for musl-64 bits platforms
#[cfg(all(target_env = "musl", target_pointer_width = "64"))]
#[global_allocator]
static ALLOC: jemallocator::Jemalloc = jemallocator::Jemalloc;

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
            let json_file = File::open(&json_file_path);
            match json_file {
                Ok(mut data_file) => {
                    let mut json_content = String::new();
                    data_file
                        .read_to_string(&mut json_content)
                        .expect("File read failed");
                    config =
                        serde_json::from_str(&json_content).expect("JSON was not well-formatted");
                }
                Err(_) => {
                    log::error!("config.json not found now checking url");
                    // check from env url
                    let config_url = env::var("CONFIG_URL");
                    match config_url {
                        Ok(url) => {
                            log::info!("Using config from url {:?}", url);
                            // set request using awc
                            // add rust tls in awc request
                            let mut res = Client::new()
                                .get(url)
                                .send()
                                .await
                                .expect("Failed to get response");

                            let body = res.body().await.expect("Failed to get response body");

                            config =
                                serde_json::from_slice(&body).expect("JSON was not well-formatted");
                        }
                        Err(_) => {
                            log::error!("config.json not found and url not set");
                            panic!("config.json not found and url not set");
                        }
                    }
                }
            }
        }
    }

    // initiate global config router
    config.router();
    GLOBAL_CONFIG.set(config.clone()).unwrap();

    // get env port
    let port = env::var("PORT").unwrap_or_else(|_| config.port.to_string());
    let host = env::var("PROXY_URL").unwrap_or_else(|_| config.proxy.clone());

    HttpServer::new(move || {
        App::new()
            .wrap(actix_web::middleware::Logger::default())
            .wrap(mw::limiter::RequestLimiter)
            .wrap(mw::auth_parse::CheckAuth)
            .app_data(web::Data::new(AppData::new(
                Client::default(),
                host.clone(),
            )))
            .default_service(web::route().to(reverse_proxy::forward))
    })
    .bind(format!("0.0.0.0:{}", port))?
    .run()
    .await
}
