use base64::{engine::general_purpose::STANDARD_NO_PAD, Engine as _};
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
            // path into all Limiter HashMap
            let mut new_hash: HashMap<String, Limiter> = HashMap::new();
            for (method, limiter) in hash.iter() {
                let code = STANDARD_NO_PAD.encode(format!("{:?}{:?}", path, method));
                new_hash.insert(
                    method.clone(),
                    Limiter::new(
                        Some(code),
                        limiter.max,
                        limiter.duration,
                        limiter.jwt_validation.clone(),
                        limiter.disabled,
                    ),
                );
            }
            router.add(path, new_hash);
        }
        // get or init
        ROUTE_LIMITER.get_or_init(|| router.clone());
        router
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Limiter {
    // default to global string
    #[serde(default = "default_code")]
    pub code: String,
    pub max: u32,
    pub duration: u64,
    pub jwt_validation: JwtValidation,
    #[serde(default)]
    pub disabled: bool,
}

fn default_code() -> String {
    String::from("global")
}
impl Limiter {
    pub fn new(
        code: Option<String>,
        max: u32,
        duration: u64,
        jwt_validation: JwtValidation,
        disabled: bool,
    ) -> Self {
        Limiter {
            // set code to global from
            code: code.ok_or("global").unwrap(),
            max,
            duration,
            jwt_validation,
            disabled,
        }
    }
}
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JwtValidation {
    pub validate: bool,
    pub params: Vec<String>,
}
