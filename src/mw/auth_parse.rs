use std::future::{ready, Ready};

use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    Error, HttpMessage, HttpResponse,
};
use awc::body::EitherBody;
use base64::{
    alphabet,
    engine::{self, general_purpose},
    Engine as _,
};
use futures_util::{future::LocalBoxFuture, FutureExt, TryFutureExt};
use serde_json::Value;

// There are two steps in middleware processing.
// 1. Middleware initialization, middleware factory gets called with
//    next service in chain as parameter.
// 2. Middleware's call method gets called with normal request.
pub struct CheckAuth;

// Middleware factory is `Transform` trait
// `S` - type of the next service
// `B` - type of response's body
impl<S, B> Transform<S, ServiceRequest> for CheckAuth
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type InitError = ();
    type Transform = CheckAuthMiddleware<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(CheckAuthMiddleware { service }))
    }
}

pub struct CheckAuthMiddleware<S> {
    service: S,
}

// Function to decode JWT and extract the `sub` claim without validation
fn decode_jwt_and_get_sub(token: &str) -> Option<Value> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }

    let payload = parts[1];

    let decoded = engine::GeneralPurpose::new(&alphabet::URL_SAFE, general_purpose::NO_PAD)
        .decode(payload.as_bytes())
        .unwrap();

    let payload_json: Value = serde_json::from_slice(&decoded).ok()?;

    // return Value payload
    Some(payload_json)
}

impl<S, B> Service<ServiceRequest> for CheckAuthMiddleware<S>
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
        match req.headers().get("Authorization").and_then(|value| {
            value.to_str().ok().and_then(|value| {
                let parts: Vec<&str> = value.split_whitespace().collect();
                if parts.len() == 2 && parts[0] == "Bearer" {
                    Some(parts[1])
                } else {
                    None
                }
            })
        }) {
            Some(token) => {
                let jwt_value = decode_jwt_and_get_sub(token);
                match jwt_value {
                    Some(jwt_value) => {
                        req.extensions_mut().insert(jwt_value);
                        self.service
                            .call(req)
                            .map_ok(ServiceResponse::map_into_left_body)
                            .boxed_local()
                    }
                    None => Box::pin(async {
                        Ok(req.into_response(
                            HttpResponse::Unauthorized().finish().map_into_right_body(),
                        ))
                    }),
                }
            }
            None => Box::pin(async {
                Ok(req.into_response(HttpResponse::Unauthorized().finish().map_into_right_body()))
            }),
        }
    }
}
