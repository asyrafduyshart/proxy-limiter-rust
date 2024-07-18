use std::time::Duration;

use actix_web::{error, web, Error, HttpRequest, HttpResponse};
use url::Url;

use crate::{domain::config, AppData};

pub async fn forward(
    req: HttpRequest,
    payload: web::Payload,
    app_data: web::Data<AppData>,
) -> Result<HttpResponse, Error> {
    let client = app_data.client.clone();
    let host = app_data.host.clone();
    let config = config::GLOBAL_CONFIG.get().unwrap();

    let mut new_url = match Url::parse(&host.as_str()) {
        Ok(url) => url,
        Err(err) => return Err(error::ErrorInternalServerError(err)),
    };

    new_url.set_path(req.uri().path());
    new_url.set_query(req.uri().query());

    let forwarded_req: awc::ClientRequest = client
        .request_from(new_url.as_str(), req.head())
        .timeout(Duration::from_secs(config.timeout));

    let res = match forwarded_req.send_stream(payload).await {
        Ok(res) => res,
        Err(err) => {
            log::error!("Error forwarding request: {:?}", err);
            return Err(error::ErrorInternalServerError(err));
        }
    };

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
