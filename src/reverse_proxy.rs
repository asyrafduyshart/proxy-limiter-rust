use actix_web::{error, web, Error, HttpRequest, HttpResponse};
use url::Url;

use crate::AppData;

pub async fn forward(
    req: HttpRequest,
    payload: web::Payload,
    app_data: web::Data<AppData>,
) -> Result<HttpResponse, Error> {
    let client = app_data.client.clone();
    let config = app_data.config.clone();

    let mut new_url = match Url::parse(config.proxy.as_str()) {
        Ok(url) => url,
        Err(err) => return Err(error::ErrorInternalServerError(err)),
    };

    new_url.set_path(req.uri().path());
    new_url.set_query(req.uri().query());

    let forwarded_req = client.request_from(new_url.as_str(), req.head());

    let res = forwarded_req
        .send_stream(payload)
        .await
        .map_err(error::ErrorInternalServerError)?;

    let mut client_resp = HttpResponse::build(res.status());

    for (header_name, header_value) in res
        .headers()
        .iter()
        .filter(|(h, _)| *h != "content-encoding")
    {
        client_resp.append_header((header_name.clone(), header_value.clone()));
    }

    Ok(client_resp.streaming(res))
}
