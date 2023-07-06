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

#![feature(lazy_cell)]
#![feature(adt_const_params)]
#![allow(incomplete_features)]
extern crate core;

use std::env;
use std::net::SocketAddr;

use anyhow::Context as _;
use hmac::digest::KeyInit;
use hmac::Hmac;
use hyper::client::HttpConnector;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Client, Request, Response, Server};
use hyper_tls::HttpsConnector;

use crate::hypixel::Rule;
use crate::util::Obscure;

pub mod hypixel;
pub mod meta;
pub mod mojang;
pub mod util;

#[derive(Debug)]
pub struct RequestContext {
    redis_client: Obscure<redis::aio::ConnectionManager, "ConnectionManager">,
    request: Request<Body>,
}

#[derive(Debug)]
pub struct GlobalApplicationContext {
    client: Client<HttpsConnector<HttpConnector>>,
    hypixel_token: Obscure<String>,
    port: u16,
    rules: Vec<Rule>,
    allow_anonymous: bool,
    // Use sha384 to prevent against length extension attacks
    key: Hmac<sha2::Sha384>,
    redis_url: Obscure<String>,
}

fn make_error(status_code: u16, error_text: &str) -> anyhow::Result<Response<Body>> {
    Ok(Response::builder()
        .status(status_code)
        .body(format!("{} {}", status_code, error_text).into())?)
}

async fn respond_to_meta(req: RequestContext, meta_path: &str) -> anyhow::Result<Response<Body>> {
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

async fn respond_to(mut context: RequestContext) -> anyhow::Result<Response<Body>> {
    let path = &context.request.uri().path().to_owned();
    if path == "/" {
        return Ok(Response::builder()
            .status(302)
            .header("Location", "https://github.com/NotEnoughUpdates/ursa-minor")
            .body(Body::empty())?);
    }
    if let Some(meta_path) = path.strip_prefix("/_meta/") {
        return respond_to_meta(context, meta_path).await;
    }

    if let Some(hypixel_path) = path.strip_prefix("/v1/hypixel/") {
        let (save, _principal) = require_login!(context);
        if let Some(resp) = hypixel::respond_to(&mut context, hypixel_path).await? {
            return save.save_to(resp);
        }
    }
    return make_error(404, format!("Unknown request path {}", path).as_str());
}

async fn wrap_error(context: RequestContext) -> anyhow::Result<Response<Body>> {
    match respond_to(context).await {
        Ok(x) => Ok(x),
        Err(e) => {
            let error_id = uuid::Uuid::new_v4();
            eprintln!("Error id: {error_id}:");
            eprintln!("{e:#?}");
            Ok(Response::builder()
                .status(500)
                .body(format!("500 Internal Error\n\nError id: {}", error_id).into())?)
        }
    }
}

fn config_var(name: &str) -> anyhow::Result<String> {
    env::var(format!("URSA_{}", name)).with_context(|| {
        format!(
            "Could not find {} expected to be found in the environment at URSA_{}",
            name, name
        )
    })
}

#[allow(non_upper_case_globals)]
static global_application_config: std::sync::LazyLock<GlobalApplicationContext> =
    std::sync::LazyLock::new(|| init_config().unwrap());

fn init_config() -> anyhow::Result<GlobalApplicationContext> {
    if let Ok(path) = dotenv::dotenv() {
        println!("Loaded dotenv from {}", path.to_str().unwrap_or("?"));
    }
    let hypixel_token = config_var("HYPIXEL_TOKEN")?;
    let allow_anonymous = config_var("ANONYMOUS").unwrap_or("false".to_owned()) == "true";
    let rules = config_var("RULES")?
        .split(':')
        .map(|it| {
            std::fs::read(it)
                .map_err(anyhow::Error::from)
                .and_then(|it| {
                    serde_json::from_slice::<hypixel::Rule>(&it).map_err(anyhow::Error::from)
                })
        })
        .collect::<Result<Vec<Rule>, _>>()?;
    let port = config_var("PORT")?
        .parse::<u16>()
        .with_context(|| "Could not parse port at URSA_PORT")?;
    let secret = config_var("SECRET")?;
    let https = HttpsConnector::new();
    let client = Client::builder().build::<_, Body>(https);
    let redis_url = config_var("REDIS_URL")?;
    Ok(GlobalApplicationContext {
        client,
        port,
        hypixel_token: Obscure(hypixel_token),
        rules,
        allow_anonymous,
        key: Hmac::new_from_slice(secret.as_bytes())?,
        redis_url: Obscure(redis_url),
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("Ursa minor rises above the sky!");
    println!(
        "Launching with configuration: {:#?}",
        *global_application_config
    );
    let addr = SocketAddr::from(([127, 0, 0, 1], global_application_config.port));
    let redis_client = redis::Client::open(global_application_config.redis_url.clone())?;
    let managed = redis::aio::ConnectionManager::new(redis_client).await?;
    let service = make_service_fn(|_conn| {
        let client = managed.clone();
        async {
            Ok::<_, anyhow::Error>(service_fn(move |req| {
                wrap_error(RequestContext {
                    redis_client: Obscure(client.clone()),
                    request: req,
                })
            }))
        }
    });
    let server = Server::bind(&addr).serve(service);
    println!("Now listening at {}", addr);
    let mut s = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        _ = s.recv() => {
            println!("Terminated! It's time to say goodbye.");
        }
        it = tokio::signal::ctrl_c() => {
            it?;
            println!("Interrupted! It's time to say goodbye.");
        }
        it = server =>{
            it?;
        }
    }
    Ok(())
}
