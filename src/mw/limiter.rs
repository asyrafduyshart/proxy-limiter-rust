use route_recognizer::Match;
use route_recognizer::Router;

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
    sub: String,
    path: String,
    method: String,
}
impl TokenHash {
    fn new(sub: &str, path: &str, method: &str) -> Self {
        TokenHash {
            sub: sub.to_string(),
            path: path.to_string(),
            method: method.to_string(),
        }
    }
}
// There are two steps in middleware processing.
// 1. Middleware initialization, middleware factory gets called with
//    next service in chain as parameter.
// 2. Middleware's call method gets called with normal request.
pub struct RequestLimiter {
    pub config: domain::config::Config,
    pub router: Router<HashMap<String, Limiter>>,
}

impl RequestLimiter {
    pub fn new(config: domain::config::Config) -> Self {
        RequestLimiter {
            config: config.clone(),
            router: config.router(),
        }
    }
}

static RATE_LIMITERS: OnceLock<
    Mutex<HashMap<String, RateLimiter<TokenHash, DashMapStateStore<TokenHash>, QuantaClock>>>,
> = OnceLock::new();

struct GlobalLimiter {
    token: TokenHash,
}
impl GlobalLimiter {
    fn new(token: TokenHash) -> Self {
        GlobalLimiter { token }
    }
    fn check(&self) -> Result<(), Box<dyn std::error::Error>> {
        let limiters = RATE_LIMITERS.get_or_init(|| Mutex::new(HashMap::new()));
        let _config = domain::config::GLOBAL_CONFIG.get().unwrap();
        let routes = ROUTE_LIMITER.get().unwrap();
        let mut limiters = limiters.lock().unwrap();

        // set to global limiter

        let route_finder: Result<Match<&HashMap<String, Limiter>>, _> =
            routes.recognize(&self.token.path);

        match route_finder {
            // set type to Match<&HashMap<String, Limiter>>
            Ok(res) => {
                let data: &HashMap<String, Limiter> = res.handler();
                // loop through data
                let res = data.get(&self.token.method);
                match res {
                    Some(limiter) => {
                        println!("limiter found {:?}", limiter);
                    }
                    None => println!("Method not found"),
                }
            }
            Err(_) => println!("Route not found"),
        }

        let limiter = limiters.get(&self.token.path);
        match limiter {
            Some(limiter) => {
                // return limiter found
                // limiter.shrink_to_fit();
                match limiter.check_key(&self.token) {
                    Ok(()) => return Ok(()),
                    Err(_) => {
                        return Err("Rate limit exceeded".into());
                    }
                }
            }
            None => {
                // return error limiter not found
                let quota = Quota::per_minute(NonZeroU32::new(5).unwrap());
                let clock = QuantaClock::default();
                let keyed: RateLimiter<TokenHash, DashMapStateStore<TokenHash>, QuantaClock> =
                    RateLimiter::dashmap_with_clock(quota, &clock);

                limiters.insert(self.token.path.clone(), keyed);
                return Ok(());
            }
        };
    }
}

// Middleware factory is `Transform` trait
// `S` - type of the next service
// `B` - type of response's body
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
    pub service: S,
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
        let path = req.path().to_string();
        let method = req.method().to_string();
        let sub = match req.extensions().get::<Value>() {
            Some(val) => val.get("sub").unwrap().to_string(),
            None => "".to_string(),
        };

        match GlobalLimiter::new(TokenHash::new(&sub, &path, &method)).check() {
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
