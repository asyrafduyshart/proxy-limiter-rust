use std::{
    collections::HashMap,
    future::{ready, Ready},
    num::NonZeroU32,
    sync::{Mutex, OnceLock},
};

use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    Error, HttpMessage, HttpResponse,
};
use awc::body::EitherBody;
use futures_util::{future::LocalBoxFuture, FutureExt, TryFutureExt};
use governor::{clock::QuantaClock, state::keyed::DashMapStateStore, Quota, RateLimiter};
use serde_json::Value;

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
    pub rate: u32,
}

impl RequestLimiter {
    pub fn new(rate: u32) -> Self {
        RequestLimiter { rate }
    }
}

static RATE_LIMITERS: OnceLock<
    Mutex<HashMap<String, RateLimiter<TokenHash, DashMapStateStore<TokenHash>, QuantaClock>>>,
> = OnceLock::new();

pub fn check_limiter_for_user(
    sub: &str,
    path: &str,
    method: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let limiters = RATE_LIMITERS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut limiters = limiters.lock().unwrap();
    let limiter = limiters.get(sub);
    match limiter {
        Some(limiter) => {
            // return limiter found
            let key = TokenHash::new(sub, path, method);
            // limiter.shrink_to_fit();
            match limiter.check_key(&key) {
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

            limiters.insert(sub.to_owned(), keyed);
            return Ok(());
        }
    };
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
        let path = req.path().to_string();
        let method = req.method().to_string();
        let sub = match req.extensions().get::<Value>() {
            Some(val) => val.get("sub").unwrap().to_string(),
            None => "".to_string(),
        };

        // generate token hash
        match check_limiter_for_user(&sub, &path, &method) {
            Ok(_) => self
                .service
                .call(req)
                .map_ok(ServiceResponse::map_into_left_body)
                .boxed_local(),
            Err(_e) => Box::pin(async {
                Ok(req.into_response(
                    HttpResponse::TooManyRequests()
                        .finish()
                        .map_into_right_body(),
                ))
            }),
        }
    }
}
