use log::info;
use route_recognizer::Match;

use std::{
    collections::HashMap,
    future::{ready, Ready},
    hash::Hash,
    num::NonZeroU32,
    sync::{Mutex, OnceLock},
};

use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    Error, HttpMessage,
};
use awc::body::EitherBody;
use futures_util::{future::LocalBoxFuture, FutureExt, TryFutureExt};
use governor::{clock::QuantaClock, state::keyed::DashMapStateStore, Quota, RateLimiter};
use serde_json::Value;

use crate::domain::{
    self,
    config::{Limiter, ROUTE_LIMITER},
};

#[derive(Hash, Eq, PartialEq, Debug, Clone)]
pub struct TokenHash {
    code: String,
    path: String,
    method: String,
}
impl TokenHash {
    fn new(code: &str, path: &str, method: &str) -> Self {
        TokenHash {
            code: code.to_string(),
            path: path.to_string(),
            method: method.to_string(),
        }
    }
}

pub struct RequestLimiter;

static RATE_LIMITERS: OnceLock<
    Mutex<HashMap<String, RateLimiter<TokenHash, DashMapStateStore<TokenHash>, QuantaClock>>>,
> = OnceLock::new();

struct GlobalLimiter {
    path: String,
    method: String,
    ipv4: Option<String>,
    info: Option<Value>,
}

/// Implementation of the GlobalLimiter struct.
impl GlobalLimiter {
    /// Creates a new instance of GlobalLimiter.
    ///
    /// # Arguments
    ///
    /// * `path` - A String representing the path of the limiter.
    /// * `method` - A String representing the HTTP method of the limiter.
    /// * `ipv4` - An optional String representing the IPv4 address.
    /// * `info` - An optional Value representing additional information.
    ///
    /// # Returns
    ///
    /// A new instance of GlobalLimiter.
    fn new(path: String, method: String, ipv4: Option<String>, info: Option<Value>) -> Self {
        GlobalLimiter {
            path,
            method,
            ipv4,
            info,
        }
    }

    /// Checks if the limiter is within the rate limits.
    ///
    /// # Returns
    ///
    /// - `Ok(())` if the limiter is within the rate limits.
    /// - `Err(Box<dyn std::error::Error>)` if the rate limit is exceeded.
    fn check(&self) -> Result<(), Box<dyn std::error::Error>> {
        // Get the global rate limiters
        let limiters = RATE_LIMITERS.get_or_init(|| Mutex::new(HashMap::new()));
        // Get the global configuration
        let config = domain::config::GLOBAL_CONFIG.get().unwrap();
        // Get the route limiters
        let routes = match ROUTE_LIMITER.get() {
            Some(routes) => routes,
            None => {
                eprintln!("ROUTE_LIMITER_NOT_FOUND");
                return Err("ROUTE_LIMITER_NOT_FOUND".into());
            }
        };
        // Lock the limiters
        let mut limiters = match limiters.lock() {
            Ok(lock) => lock,
            Err(_) => {
                eprintln!("RATE_LIMITER_LOCK_ERROR");
                return Err("RATE_LIMITER_LOCK_ERROR".into());
            }
        };

        // Find the limiter for the given route
        let route_finder: Result<Match<&HashMap<String, Limiter>>, _> =
            routes.recognize(&self.path);

        // Initialize variables for the limiter codes
        let mut limited_path_code = config.global_limiter.code.clone();
        let mut prefix_code = config.global_limiter.code.clone();
        let mut main_limiter = config.global_limiter.clone();

        // Check if the route limiter is found
        if let Ok(res) = route_finder {
            let data: &HashMap<String, Limiter> = res.handler();
            if let Some(limiter) = data.get(&self.method) {
                if limiter.disabled {
                    return Ok(());
                }
                main_limiter = limiter.clone();
                limited_path_code = limiter.code.clone();
            }
        }

        // Check if additional information is provided
        if let Some(info) = &self.info {
            let set = main_limiter.jwt_validation.params.concat();
            if let Some(val) = info.get(&set) {
                prefix_code = val.as_str().unwrap().to_string();
            }
        } else {
            prefix_code = self.ipv4.clone().unwrap_or_else(|| "not_found".to_string());
        }

        // Get the limiter for the limited path code
        let limiter = limiters.get(&limited_path_code);
        // Create a token for rate limiting
        let token = TokenHash::new(&prefix_code, &self.path, &self.method);

        info!("{:?}", token);
        // Check if the limiter exists
        if let Some(limiter) = limiter {
            // print limiter
            match limiter.check_key(&token) {
                Ok(()) => {
                    return Ok(());
                }
                Err(_e) => {
                    return Err("RATE_LIMIT_EXCEEDED".into());
                }
            }
        } else {
            // Create a new limiter and insert it into the limiters map
            let quota = Quota::per_minute(NonZeroU32::new(main_limiter.max).unwrap());
            let clock = QuantaClock::default();
            let keyed: RateLimiter<TokenHash, DashMapStateStore<TokenHash>, QuantaClock> =
                RateLimiter::dashmap_with_clock(quota, &clock);

            limiters.insert(limited_path_code, keyed);
            return Ok(());
        }
    }
}

impl<S, B> Transform<S, ServiceRequest> for RequestLimiter
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type InitError = ();
    type Transform = RequestLimiterMiddleware<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(RequestLimiterMiddleware { service }))
    }
}

pub struct RequestLimiterMiddleware<S> {
    service: S,
}

impl<S, B> Service<ServiceRequest> for RequestLimiterMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let path: String = req.path().to_string();
        let method = req.method().to_string();
        let code = req.extensions().get::<Value>().cloned();
        let ipv4 = req
            .connection_info()
            .realip_remote_addr()
            .map(|ip| ip.to_string());

        match GlobalLimiter::new(path.clone(), method.clone(), ipv4, code).check() {
            Ok(_) => {
                return self
                    .service
                    .call(req)
                    .map_ok(ServiceResponse::map_into_left_body)
                    .boxed_local();
            }
            Err(_e) => {
                return Box::pin(async {
                    Ok(req.into_response(
                        actix_web::HttpResponse::TooManyRequests()
                            .finish()
                            .map_into_right_body(),
                    ))
                });
            }
        }
    }
}
