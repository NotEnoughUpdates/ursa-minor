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

use std::collections::HashMap;

use hyper::{Body, Response};
use serde::Serialize;

use crate::{global_application_config, make_error, require_login, RequestContext};

pub const BUILD_VERSION: &str = env!("GIT_HASH");

#[derive(Serialize)]
struct Stats {
    request_total: HashMap<String, u64>,
}

async fn respond_to_statistics(mut req: RequestContext) -> anyhow::Result<Response<Body>> {
    let mut pipe = redis::pipe();
    for rule in &global_application_config.rules {
        pipe.get(rule.accumulated_statistics_key());
    }
    let response: Vec<Option<u64>> = pipe.query_async(&mut req.redis_client.0).await?;
    let mut request_total = HashMap::new();
    for (value, rule) in response.iter().zip(global_application_config.rules.iter()) {
        request_total.insert(rule.http_path.clone(), value.unwrap_or(0));
    }
    return Ok(Response::builder()
        .header("content-type", "application/json")
        .body(serde_json::to_string(&Stats { request_total })?.into())?);
}

pub async fn respond_to_meta(
    req: RequestContext,
    meta_path: &str,
) -> anyhow::Result<Response<Body>> {
    if meta_path == "version" {
        return Ok(Response::builder()
            .status(200)
            .body(debug_string().into())?);
    }
    let (save, principal) = require_login!(req);
    let response = if meta_path == "principal" {
        Response::builder()
            .status(200)
            .body(format!("{principal:#?}").into())?
    } else if meta_path == "stats" {
        respond_to_statistics(req).await?
    } else {
        make_error(404, format!("Unknown meta request {meta_path}").as_str())?
    };
    save.save_to(response)
}

pub fn debug_string() -> String {
    format!(
        "ursa-minor {} https://github.com/NotEnoughUpdates/ursa-minor/\nfeatures: {}",
        BUILD_VERSION,
        crate::built_info::FEATURES_STR
    )
}
