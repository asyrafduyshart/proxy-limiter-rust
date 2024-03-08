use base64::{alphabet, engine, engine::general_purpose, Engine as _};
use governor::clock::QuantaClock;
use governor::state::keyed::DashMapStateStore;
use governor::{Quota, RateLimiter};

use serde_json::Value;
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::{Mutex, OnceLock};

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

static RATE_LIMITERS: OnceLock<
    Mutex<HashMap<String, RateLimiter<TokenHash, DashMapStateStore<TokenHash>, QuantaClock>>>,
> = OnceLock::new();

// Create a rate limiter for a given user
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
            let keyed: RateLimiter<TokenHash, DashMapStateStore<TokenHash>, QuantaClock> =
                RateLimiter::dashmap_with_clock(quota, &clock);

            limiters.insert(sub.to_owned(), keyed);
            return Ok(());
        }
    };
}

// Function to decode JWT and extract the `sub` claim without validation
pub fn decode_jwt_and_get_sub(token: &str) -> Option<Value> {
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
