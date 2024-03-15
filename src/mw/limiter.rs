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
    path: String,
    method: String,
    info: Option<Value>,
}
impl GlobalLimiter {
    fn new(path: String, method: String, info: Option<Value>) -> Self {
        GlobalLimiter { path, method, info }
    }
    fn check(&self) -> Result<(), Box<dyn std::error::Error>> {
        let limiters = RATE_LIMITERS.get_or_init(|| Mutex::new(HashMap::new()));
        let config = domain::config::GLOBAL_CONFIG.get().unwrap();
        let routes = ROUTE_LIMITER.get().unwrap();
        let mut limiters = limiters.lock().unwrap();

        let route_finder: Result<Match<&HashMap<String, Limiter>>, _> =
            routes.recognize(&self.path);

        let mut limited_path_code = config.global_limiter.code.clone();
        let mut prefix_code = config.global_limiter.code.clone();
        let mut main_limiter = config.global_limiter.clone();

        match route_finder {
            // set type to Match<&HashMap<String, Limiter>>
            Ok(res) => {
                let data: &HashMap<String, Limiter> = res.handler();
                // loop through data
                let res = data.get(&self.method);
                match res {
                    Some(limiter) => {
                        main_limiter = limiter.clone();
                        // println!("Limiter found: {:?}", limiter);
                        limited_path_code = limiter.code.clone();
                        match &self.info {
                            Some(info) => {
                                // join all data sub from limiter into one string
                                let set = limiter.jwt_validation.params.concat();
                                // check if info contains all set or set default to string
                                match info.get(&set) {
                                    Some(val) => {
                                        prefix_code = val.as_str().unwrap().to_string();
                                    }
                                    None => {}
                                }
                            }
                            None => {
                                prefix_code = config.global_limiter.code.clone();
                            }
                        }
                        // limited_path = limiter.max
                        // check sub if need jwt validation and sub is not empty string
                        // if limiter.jwt_validation.validate {
                        //     // return error if jwt validation failed
                        //     return Err("JWT_VALIDATION_FAILED".into());
                        // }
                    }
                    None => println!("Method not found"),
                }
            }
            Err(_) => println!("Route not found"),
        }

        let limiter = limiters.get(&limited_path_code);

        // token global limiter
        let token = TokenHash::new(&prefix_code, &self.path, &self.method);
        match limiter {
            Some(limiter) => match limiter.check_key(&token) {
                Ok(()) => return Ok(()),
                Err(_) => {
                    return Err("RATE_LIMIT_EXCEEDED".into());
                }
            },
            None => {
                // return error limiter not found
                let quota =
                    Quota::with_period(std::time::Duration::from_secs(main_limiter.duration))
                        .unwrap()
                        .allow_burst(NonZeroU32::new(main_limiter.max).unwrap());
                let clock = QuantaClock::default();
                let keyed: RateLimiter<TokenHash, DashMapStateStore<TokenHash>, QuantaClock> =
                    RateLimiter::dashmap_with_clock(quota, &clock);

                limiters.insert(limited_path_code, keyed);
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
        let path: String = req.path().to_string();
        let method = req.method().to_string();
        let code = req.extensions().get::<Value>().cloned();

        match GlobalLimiter::new(path.clone(), method.clone(), code).check() {
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
