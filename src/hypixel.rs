use hyper::{Body, Method, Request, Response};
use serde::Deserialize;
use url::Url;

use crate::{global_application_config, make_error};
use crate::util::UrlForRequest;

#[derive(Deserialize, Debug)]
pub struct Rule {
    /// The path of this endpoint in our api.
    #[serde(rename = "http-path")]
    pub http_path: String,
    /// The path of this endpoint in the hypixel API.
    #[serde(rename = "hypixel-path")]
    pub hypixel_path: String,
    /// Additional path segments will be transformed into query arguments with names accordion to this array.
    /// If there are extra or missing arguments this endpoint errors
    #[serde(rename = "query-arguments")]
    pub query_arguments: Vec<String>,

    // TODO: filters
}

pub async fn respond_to(path: &str) -> anyhow::Result<Option<Response<Body>>> {
    for rule in &global_application_config.rules {
        if let Some(prefix) = path.strip_prefix(&rule.http_path) {
            let mut parts = prefix.split("/").filter(|it| !it.is_empty());
            let mut query_parts: Vec<(String, String)> = Vec::with_capacity(rule.query_arguments.len());
            for query_argument in &rule.query_arguments {
                let Some(next_part) = parts.next() else {
                    return make_error(400, format!("Missing query argument {}", query_argument).as_str()).map(Some);
                };
                query_parts.push((query_argument.clone(), next_part.to_owned()));
            }
            if let Some(extra) = parts.next() {
                return make_error(400, format!("Superfluous query argument {:?}", extra).as_str()).map(Some);
            }
            let url = Url::parse_with_params(rule.hypixel_path.as_str(), query_parts)?;
            println!("Proxying request for {url}");
            let hypixel_request = Request::builder()
                .url(url)?
                .method(Method::GET)
                .header("API-Key", &global_application_config.hypixel_token.0)
                .body(Body::empty())?;
            let hypixel_response = global_application_config.client.request(hypixel_request).await?;
            // TODO: add temporary global backoff when hitting an error (especially 429)
            if hypixel_response.status().as_u16() != 200 {
                return make_error(502, "Failed to request hypixel upstream").map(Some);
            }
            return Ok(Some(Response::builder()
                .header("Age", "0")
                .header("Cache-Control", "public, s-maxage=60, max-age=300")
                .header("Content-Type", "application/json")
                .body(hypixel_response.into_body())?));
        }
    }
    Ok(None)
}
