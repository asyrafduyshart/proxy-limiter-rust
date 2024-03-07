use actix_web::{middleware, web, App, HttpRequest, HttpResponse, HttpServer};
use base64::{alphabet, engine, engine::general_purpose, Engine as _};
use governor::clock::QuantaClock;
use governor::state::keyed::DashMapStateStore;
use governor::{Quota, RateLimiter};

use serde_json::Value;
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::{Mutex, OnceLock};

// JWT claim
// #[derive(Debug, Deserialize)]
// struct Claims {
//     sub: String,
//     exp: usize,
// }
#[derive(Hash, Eq, PartialEq, Debug, Clone)]
struct Rustacean {
    name: String,
    country: String,
}
impl Rustacean {
    fn new(name: &str, country: &str) -> Self {
        Rustacean {
            name: name.to_string(),
            country: country.to_string(),
        }
    }
}

// static RATE_LIMITERS: OnceLock<
//     Mutex<HashMap<String, RateLimiter<NotKeyed, InMemoryState, QuantaClock>>>,
// > = OnceLock::new();

static RATE_LIMITERS: OnceLock<
    Mutex<HashMap<String, RateLimiter<Rustacean, DashMapStateStore<Rustacean>, QuantaClock>>>,
> = OnceLock::new();

// Create a rate limiter for a given user
fn check_limiter_for_user(sub: &str) -> Result<(), Box<dyn std::error::Error>> {
    let limiters = RATE_LIMITERS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut limiters = limiters.lock().unwrap();
    let limiter = limiters.get(sub);
    match limiter {
        Some(limiter) => {
            // return limiter found
            let key = Rustacean::new("hello", "mama");
            // limiter.shrink_to_fit();
            match limiter.check_key(&key) {
                Ok(()) => {
                    return Ok(());
                }
                Err(_) => {
                    return Err("Rate limit exceeded".into());
                }
            }
        }
        None => {
            // return error limiter not found
            let quota = Quota::per_minute(NonZeroU32::new(5).unwrap());
            let clock = QuantaClock::default();
            let keyed: RateLimiter<Rustacean, DashMapStateStore<Rustacean>, QuantaClock> =
                RateLimiter::dashmap_with_clock(quota, &clock);

            limiters.insert(sub.to_owned(), keyed);
            return Ok(());
        }
    };
}

// Function to decode JWT and extract the `sub` claim without validation
fn decode_jwt_and_get_sub(token: &str) -> Option<String> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }

    let payload = parts[1];

    let decoded = engine::GeneralPurpose::new(&alphabet::URL_SAFE, general_purpose::NO_PAD)
        .decode(payload.as_bytes())
        .unwrap();

    let payload_json: Value = serde_json::from_slice(&decoded).ok()?;

    payload_json
        .get("sub")
        .and_then(|sub| sub.as_str())
        .map(|s| s.to_owned())
}

async fn proxy(req: HttpRequest, body: web::Bytes) -> HttpResponse {
    // Extract the token from the request headers
    // get full path from req
    let path = req.uri().path().to_string();
    let token = req.headers().get("Authorization").and_then(|value| {
        value.to_str().ok().and_then(|value| {
            let parts: Vec<&str> = value.split_whitespace().collect();
            if parts.len() == 2 && parts[0] == "Bearer" {
                Some(parts[1])
            } else {
                None
            }
        })
    });

    // Check if the token is present
    let sub = match token {
        Some(token) => {
            let tkn = decode_jwt_and_get_sub(token);
            match tkn {
                Some(tkn) => match check_limiter_for_user(&tkn) {
                    Ok(()) => tkn,
                    Err(_e) => {
                        return HttpResponse::TooManyRequests().finish();
                    }
                },
                None => {
                    return HttpResponse::Unauthorized().finish();
                }
            }
        }
        None => {
            return HttpResponse::Unauthorized().finish();
        }
    };

    HttpResponse::Ok().finish()
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    HttpServer::new(|| {
        App::new().wrap(middleware::Logger::default()).service(
            web::resource("/proxy")
                .route(web::get().to(proxy))
                .route(web::post().to(proxy))
                .route(web::put().to(proxy))
                .route(web::delete().to(proxy)),
        )
    })
    .bind("127.0.0.1:9080")?
    .run()
    .await
}
