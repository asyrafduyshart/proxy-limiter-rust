use actix_web::{error, web, Error, HttpRequest, HttpResponse};
use url::Url;

use crate::{
    limiter::{check_limiter_for_user, decode_jwt_and_get_sub},
    AppData,
};

// crate limiter

pub async fn forward(
    req: HttpRequest,
    payload: web::Payload,
    app_data: web::Data<AppData>,
) -> Result<HttpResponse, Error> {
    let client = app_data.client.clone();
    // get Authroization header
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
            let tkn = decode_jwt_and_get_sub(token);
            match tkn {
                Some(tkn) => match check_limiter_for_user(
                    &tkn,
                    &req.uri().path().to_string(),
                    &req.method().to_string(),
                ) {
                    Ok(()) => (),
                    Err(_e) => {
                        return Ok(HttpResponse::TooManyRequests().finish());
                    }
                },
                None => {
                    return Ok(HttpResponse::Unauthorized().finish());
                }
            }
        }
        None => {}
    }

    // let host = req.headers().get("host").unwrap().to_str().unwrap();
    let mut new_url = Url::parse(&format!("http://{}", "httpbin.org")).unwrap();
    new_url.set_path(req.uri().path());
    new_url.set_query(req.uri().query());

    // TODO: This forwarded implementation is incomplete as it only handles the inofficial
    // X-Forwarded-For header but not the official Forwarded one.
    let forwarded_req = client
        .request_from(new_url.as_str(), req.head())
        .no_decompress();
    let forwarded_req = match req.head().peer_addr {
        Some(addr) => forwarded_req.insert_header(("x-forwarded-for", format!("{}", addr.ip()))),
        None => forwarded_req,
    };

    let res = forwarded_req
        .send_stream(payload)
        .await
        .map_err(error::ErrorInternalServerError)?;

    let mut client_resp = HttpResponse::build(res.status());
    // Remove `Connection` as per
    // https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Connection#Directives
    for (header_name, header_value) in res.headers().iter().filter(|(h, _)| *h != "connection") {
        client_resp.append_header((header_name.clone(), header_value.clone()));
    }

    Ok(client_resp.streaming(res))
}
