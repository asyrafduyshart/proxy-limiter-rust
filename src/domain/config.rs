use route_recognizer::Router;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::OnceLock};

pub static ROUTE_LIMITER: OnceLock<Router<HashMap<String, Limiter>>> = OnceLock::new();

pub static GLOBAL_CONFIG: OnceLock<Config> = OnceLock::new();

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Config {
    pub port: u16,
    pub proxy: String,
    pub timeout: u64,
    pub global_limiter: Limiter,
    pub limiters: HashMap<String, HashMap<String, Limiter>>,
}

impl Config {
    pub fn router(&self) -> Router<HashMap<String, Limiter>> {
        let mut router: Router<HashMap<String, Limiter>> = Router::new();
        for (path, hash) in self.limiters.iter() {
            router.add(path, hash.clone());
        }
        // get or init
        ROUTE_LIMITER.get_or_init(|| router.clone());
        router
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Limiter {
    pub max: u64,
    pub duration: u64,
    pub jwt_validation: JwtValidation,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JwtValidation {
    pub validate: bool,
    pub params: Vec<String>,
}
