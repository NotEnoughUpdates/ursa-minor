use std::str::FromStr;
use std::sync::Arc;

use hyper::{Body, Method, Request, Response, Uri};
use serde::Deserialize;
use url::Url;

use crate::{Context, make_error};

#[derive(Deserialize)]
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

pub async fn respond_to(arc: Arc<Context>, path: &str) -> anyhow::Result<Option<Response<Body>>> {
    for rule in &arc.rules {
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
            // Sadly need to use Url for url encoding, since hypers uri does not have that capability
            let url = Url::parse_with_params(rule.hypixel_path.as_str(), query_parts)?;
            let hypixel_request = Request::builder()
                .uri(Uri::from_str(url.as_str())?)
                .method(Method::GET)
                .header("API-Key", &arc.hypixel_token)
                .body(Body::empty())?;
            let hypixel_response = arc.client.request(hypixel_request).await?;
            return Ok(Some(Response::builder()
                .header("Age", "0")
                .header("Cache-Control", "public, s-maxage=60, max-age=300")
                .header("Content-Type", "application/json")
                .body(hypixel_response.into_body())?));
        }
    }
    Ok(None)
}
