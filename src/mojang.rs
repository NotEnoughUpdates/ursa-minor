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

use std::time::SystemTime;

use anyhow::bail;
use hyper::{Body, Request, Response};
use hyper::body::Buf;
use hyper::http::HeaderValue;
use jwt::{SignWithKey, VerifyWithKey};
use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;

use crate::{global_application_config, make_error, RequestContext};
use crate::util::{MillisecondTimestamp, UrlForRequest};

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct JWTPrincipal {
    pub id: Uuid,
    pub name: String,
    pub valid_until: MillisecondTimestamp,
    pub valid_since: MillisecondTimestamp,
}

#[derive(Deserialize, Clone, Debug)]
pub struct MojangUser {
    pub id: Uuid,
    pub name: String,
}

#[must_use]
pub enum SaveOnExit {
    DontSave,
    SaveExpires {
        timestamp: MillisecondTimestamp,
    },
    Save {
        principal: JWTPrincipal,
    },
}

impl SaveOnExit {
    pub fn save_to(&self, mut response: Response<Body>) -> anyhow::Result<Response<Body>> {
        let headers = response.headers_mut();
        if let SaveOnExit::Save { principal } = self {
            let signed = SignWithKey::sign_with_key(principal, &global_application_config.key)?;
            headers.append("x-ursa-token", HeaderValue::from_str(&signed)?);
            headers.append("x-ursa-expires", HeaderValue::from_str(&principal.valid_until.0.to_string())?);
        }
        if let SaveOnExit::SaveExpires { timestamp } = self {
            headers.append("x-ursa-expires", HeaderValue::from_str(&timestamp.0.to_string())?);
        }
        Ok(response)
    }
}
#[macro_export]
macro_rules! require_login {
    ($req:expr) => {
        match $crate::mojang::require_login(&$req).await? {
            Ok(p) => p,
            Err(response) => return Ok(response),
        }
    };
}
pub async fn require_login(req: &RequestContext) -> anyhow::Result<Result<(SaveOnExit, JWTPrincipal), Response<Body>>> {
    if global_application_config.allow_anonymous {
        return Ok(Ok((SaveOnExit::DontSave, JWTPrincipal {
            id: Uuid::from_u128(0),
            name: "CoolGuy123".to_owned(),
            valid_until: MillisecondTimestamp(u64::MAX),
            valid_since: MillisecondTimestamp(0),
        })));
    }
    match verify_existing_login(req).await {
        Err(_) => {
            return Ok(Err(make_error(401, "Failed to verify JWT")?));
        }
        Ok(Some(principal)) => {
            return Ok(Ok((SaveOnExit::SaveExpires { timestamp: principal.valid_until }, principal)));
        }
        Ok(_) => {
            // Ignore absent JWT tokens
        }
    }
    verify_login_attempt(req).await.map(|it| it.map(|it| (SaveOnExit::Save {
        principal: it.clone(),
    }, it)))
}

async fn verify_existing_login(req: &RequestContext) -> anyhow::Result<Option<JWTPrincipal>> {
    let Some(token) = req.request.headers().get("x-ursa-token").and_then(|it| it.to_str().ok()) else {
        return Ok(None);
    };
    let claims: JWTPrincipal = VerifyWithKey::verify_with_key(token, &global_application_config.key)?;
    let right_now = MillisecondTimestamp::try_from(SystemTime::now())?;
    if claims.valid_since > right_now || claims.valid_until < right_now {
        bail!("JWT not valid");
    }
    Ok(Some(claims))
}


async fn verify_login_attempt(req: &RequestContext) -> anyhow::Result<Result<JWTPrincipal, Response<Body>>> {
    // this is a flawed way of doing logins, but I do not want to expend the cryptographical resources
    // to make it less flawed and everyone else does it the same way as well, and this has not become
    // a widely used attack
    let Some(username) = req.request.headers().get("x-ursa-username").and_then(|it| it.to_str().ok()) else {
        return Ok(Err(make_error(400, "Missing username to authenticate")?));
    };
    let Some(server_id) = req.request.headers().get("x-ursa-serverid").and_then(|it| it.to_str().ok()) else {
        return Ok(Err(make_error(400, "Missing serverid to authenticate")?));
    };
    let mojang_request = Request::builder()
        .url(Url::parse_with_params("https://sessionserver.mojang.com/session/minecraft/hasJoined", [("username", username), ("serverId", server_id)])?)?
        .body(Body::empty())?;
    let mojang_response = global_application_config.client.request(mojang_request).await?;
    if mojang_response.status() != 200 {
        return Ok(Err(make_error(401, "Unauthorized")?));
    }
    let buffer = hyper::body::aggregate(mojang_response).await?;
    let user = serde_json::from_reader::<_, MojangUser>(buffer.reader())?;
    let right_now = MillisecondTimestamp::try_from(SystemTime::now())?;
    Ok(Ok(JWTPrincipal {
        id: user.id,
        name: user.name,
        valid_until: right_now + global_application_config.default_token_duration,
        valid_since: right_now,
    }))
}
