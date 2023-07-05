use std::collections::BTreeMap;
use std::str::FromStr;
use anyhow::{anyhow, bail};
use hyper::{Body, Request, Response};
use serde::Deserialize;
use url::Url;
use uuid::Uuid;
use crate::{global_application_config, make_error, RequestContext};
use crate::util::UrlForRequest;
use hyper::body::Buf;
use hyper::http::HeaderValue;
use jwt::{SignWithKey, VerifyWithKey};

#[derive(Deserialize, Clone, Debug)]
pub struct MojangUserPrincipal {
    pub id: Uuid,
    pub name: String,
}

#[must_use]
pub enum SaveOnExit {
    DontSave,
    Save {
        principal: MojangUserPrincipal,
    },
}

impl SaveOnExit {
    pub fn save_to(&self, mut response: Response<Body>) -> anyhow::Result<Response<Body>> {
        if let SaveOnExit::Save { principal } = self {
            let signed = SignWithKey::sign_with_key(BTreeMap::from([
                ("id", &principal.id.to_string()),
                ("name", &principal.name),
                ("valid_since", &std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis().to_string())
            ]), &global_application_config.key)?;
            response.headers_mut().append("x-ursa-token", HeaderValue::from_str(&signed)?);
        }
        Ok(response)
    }
}
#[macro_export]
macro_rules! require_login {
    ($req:expr) => {
        match crate::mojang::require_login(&$req).await? {
            Ok(p) => p,
            Err(response) => return Ok(response),
        }
    };
}

pub async fn require_login(req: &RequestContext) -> anyhow::Result<Result<(SaveOnExit, MojangUserPrincipal), Response<Body>>> {
    if global_application_config.allow_anonymous {
        return Ok(Ok((SaveOnExit::DontSave, MojangUserPrincipal {
            id: Uuid::from_u128(0),
            name: "CoolGuy123".to_owned(),
        })));
    }
    match verify_existing_login(&req).await {
        Err(_) => {
            return Ok(Err(make_error(401, "Failed to verify JWT")?));
        }
        Ok(Some(principal)) => {
            return Ok(Ok((SaveOnExit::DontSave, principal)));
        }
        Ok(_) => {
            // Ignore absent JWT tokens
        }
    }
    verify_login_attempt(&req).await.map(|it| it.map(|it| (SaveOnExit::Save {
        principal: it.clone(),
    }, it)))
}

async fn verify_existing_login(req: &RequestContext) -> anyhow::Result<Option<MojangUserPrincipal>> {
    let Some(token) = req.request.headers().get("x-ursa-token").and_then(|it| it.to_str().ok()) else {
        return Ok(None);
    };
    let mut claims: BTreeMap<String, String> = VerifyWithKey::verify_with_key(token, &global_application_config.key)?;
    let id = claims.get("id").ok_or(anyhow!("Missing id claim")).and_then(|it| Uuid::from_str(it).map_err(anyhow::Error::new))?;
    let name = claims.remove("name").ok_or(anyhow!("Missing name claim"))?;
    let valid_since = claims.remove("valid_since").ok_or(anyhow!("Missing timestamp"))
        .and_then(|it| it.parse::<u64>().map_err(anyhow::Error::new))
        .map(std::time::Duration::from_millis)?;
    let right_now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?;
    if valid_since > right_now || valid_since + std::time::Duration::from_secs(3600) < right_now {
        bail!("JWT not valid");
    }
    return Ok(Some(MojangUserPrincipal {
        id,
        name,
    }));
}


async fn verify_login_attempt(req: &RequestContext) -> anyhow::Result<Result<MojangUserPrincipal, Response<Body>>> {
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
    let principal = serde_json::from_reader::<_, MojangUserPrincipal>(buffer.reader())?;
    Ok(Ok(principal))
}
