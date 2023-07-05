use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::Arc;
use anyhow::{anyhow, bail};
use hyper::{Body, Request, Response};
use serde::Deserialize;
use url::Url;
use uuid::Uuid;
use crate::{Context, make_error};
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
pub enum SaveOnExit<'a> {
    DontSave,
    Save {
        principal: MojangUserPrincipal,
        arc: &'a Arc<Context>,
    },
}

impl<'a> SaveOnExit<'a> {
    pub fn save_to(&self, mut response: Response<Body>) -> anyhow::Result<Response<Body>> {
        if let SaveOnExit::Save { principal, arc } = self {
            let signed = SignWithKey::sign_with_key(BTreeMap::from([
                ("id", &principal.id.to_string()),
                ("name", &principal.name),
                ("valid_since", &std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis().to_string())
            ]), &arc.key)?;
            response.headers_mut().append("x-ursa-token", HeaderValue::from_str(&signed)?);
        }
        Ok(response)
    }
}
#[macro_export]
macro_rules! require_login {
    ($arc:expr, $req:expr) => {
        match crate::mojang::require_login(&$arc, &$req).await? {
            Ok(p) => p,
            Err(response) => return Ok(response),
        }
    };
}

pub async fn require_login<'a>(arc: &'a Arc<Context>, req: &'a Request<Body>) -> anyhow::Result<Result<(SaveOnExit<'a>, MojangUserPrincipal), Response<Body>>> {
    if arc.allow_anonymous {
        return Ok(Ok((SaveOnExit::DontSave, MojangUserPrincipal {
            id: Uuid::from_u128(0),
            name: "CoolGuy123".to_owned(),
        })));
    }
    match verify_existing_login(&arc, &req).await {
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
    verify_login_attempt(&arc, &req).await.map(|it| it.map(|it| (SaveOnExit::Save {
        principal: it.clone(),
        arc,
    }, it)))
}

async fn verify_existing_login(arc: &Arc<Context>, req: &Request<Body>) -> anyhow::Result<Option<MojangUserPrincipal>> {
    let Some(token) = req.headers().get("x-ursa-token").and_then(|it| it.to_str().ok()) else {
        return Ok(None);
    };
    let mut claims: BTreeMap<String, String> = VerifyWithKey::verify_with_key(token, &arc.key)?;
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


async fn verify_login_attempt(arc: &Arc<Context>, req: &Request<Body>) -> anyhow::Result<Result<MojangUserPrincipal, Response<Body>>> {
    // this is a flawed way of doing logins, but I do not want to expend the cryptographical resources
    // to make it less flawed and everyone else does it the same way as well, and this has not become
    // a widely used attack
    let Some(username) = req.headers().get("x-ursa-username").and_then(|it| it.to_str().ok()) else {
        return Ok(Err(make_error(400, "Missing username to authenticate")?));
    };
    let Some(server_id) = req.headers().get("x-ursa-serverid").and_then(|it| it.to_str().ok()) else {
        return Ok(Err(make_error(400, "Missing serverid to authenticate")?));
    };
    let mojang_request = Request::builder()
        .url(Url::parse_with_params("https://sessionserver.mojang.com/session/minecraft/hasJoined", [("username", username), ("serverId", server_id)])?)?
        .body(Body::empty())?;
    let mojang_response = arc.client.request(mojang_request).await?;
    if mojang_response.status() != 200 {
        return Ok(Err(make_error(401, "Unauthorized")?));
    }
    let buffer = hyper::body::aggregate(mojang_response).await?;
    let principal = serde_json::from_reader::<_, MojangUserPrincipal>(buffer.reader())?;
    Ok(Ok(principal))
}
