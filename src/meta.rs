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

use hyper::{Body, Response};

use crate::{make_error, RequestContext, require_login};

pub const BUILD_VERSION: &str = env!("GIT_HASH");


pub async fn respond_to_meta(req: RequestContext, meta_path: &str) -> anyhow::Result<Response<Body>> {
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
    } else {
        make_error(404, format!("Unknown meta request {meta_path}").as_str())?
    };
    save.save_to(response)
}


pub fn debug_string() -> String {
    format!("ursa-minor {} https://github.com/NotEnoughUpdates/ursa-minor/", BUILD_VERSION)
}