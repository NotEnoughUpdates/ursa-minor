// Ursa Minor - A Hypixel API proxy
// Copyright (C) 2023 Linnea Gräf
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

use hyper::{Body, Method, Request, Response};
use redis::Pipeline;
use serde::Deserialize;
use url::Url;

use crate::mojang::JWTPrincipal;
use crate::util::UrlForRequest;
use crate::{global_application_config, make_error, RequestContext};

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

impl Rule {
    pub fn accumulated_statistics_key(&self) -> String {
        format!("hypixel:accumulated:{}", self.http_path)
    }
}

pub async fn respond_to(
    context: &mut RequestContext,
    path: &str,
    principal: JWTPrincipal,
) -> anyhow::Result<Option<Response<Body>>> {
    for rule in &global_application_config.rules {
        if let Some(prefix) = path.strip_prefix(&rule.http_path) {
            let parts = prefix
                .split('/')
                .filter(|it| !it.is_empty())
                .collect::<Vec<_>>();
            let mut query_parts: Vec<(String, String)> =
                Vec::with_capacity(rule.query_arguments.len());
            let mut part_iter = parts.iter();
            for query_argument in &rule.query_arguments {
                let Some(next_part) = part_iter.next() else {
                    return make_error(400, format!("Missing query argument {}", query_argument).as_str()).map(Some);
                };
                query_parts.push((query_argument.clone(), (*next_part).to_owned()));
            }
            if let Some(extra) = part_iter.next() {
                return make_error(
                    400,
                    format!("Superfluous query argument {:?}", extra).as_str(),
                )
                .map(Some);
            }
            let url = Url::parse_with_params(rule.hypixel_path.as_str(), query_parts)?;
            let mut diagnostics_key = String::new();
            for part in parts {
                if !diagnostics_key.is_empty() {
                    diagnostics_key.push(':');
                }
                diagnostics_key.push_str(part);
            }
            let bucket = principal.ratelimit_key();
            let mut resp = context
                .redis_client
                .send_packed_commands(
                    Pipeline::new()
                        .zincr(
                            format!("hypixel:request:{}", rule.http_path),
                            diagnostics_key,
                            1,
                        )
                        .cmd("EXPIRE")
                        .arg(&bucket)
                        .arg(global_application_config.rate_limit_lifespan.as_secs())
                        .arg("NX")
                        .incr(&bucket, 1)
                        .incr(rule.accumulated_statistics_key(), 1),
                    0,
                    3,
                )
                .await?;
            let bucket_usage = resp.remove(2);
            if let redis::Value::Int(bucket_usage_int) = bucket_usage {
                if bucket_usage_int > global_application_config.rate_limit_bucket as i64
                    && !global_application_config.allow_anonymous
                {
                    return make_error(429, "Rate limit exceeded").map(Some);
                }
            } else {
                return make_error(500, "Redis failure").map(Some);
            }

            let hypixel_request = Request::builder()
                .url(url)?
                .method(Method::GET)
                .header("API-Key", &global_application_config.hypixel_token.0)
                .body(Body::empty())?;
            let hypixel_response = global_application_config
                .client
                .request(hypixel_request)
                .await?;
            // TODO: add temporary global backoff when hitting an error (especially 429)
            if hypixel_response.status().as_u16() != 200 {
                return make_error(502, "Failed to request hypixel upstream").map(Some);
            }
            return Ok(Some(
                Response::builder()
                    .header("Age", "0")
                    .header("Cache-Control", "public, s-maxage=60, max-age=300")
                    .header("Content-Type", "application/json")
                    .body(hypixel_response.into_body())?,
            ));
        }
    }
    Ok(None)
}
