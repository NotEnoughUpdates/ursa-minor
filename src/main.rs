extern crate core;

use std::env;
use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context as _};
use hyper::{Body, Client, Request, Response, Server};
use hyper::client::HttpConnector;
use hyper::service::{make_service_fn, service_fn};
use hyper_tls::HttpsConnector;

use crate::hypixel::Rule;

pub mod hypixel;
pub mod meta;

pub struct Context {
    client: Client<HttpsConnector<HttpConnector>>,
    hypixel_token: String,
    rules: Vec<hypixel::Rule>,
}

fn make_error(status_code: u16, error_text: &str) -> anyhow::Result<Response<Body>> {
    return Ok(Response::builder()
        .status(status_code)
        .body(format!("{} {}", status_code, error_text).into())?);
}

async fn respond_to_meta(_arc: Arc<Context>, _req: Request<Body>, meta_path: &str) -> anyhow::Result<Response<Body>> {
    return make_error(404, format!("Unknown meta request {}", meta_path).as_str());
}

async fn respond_to(arc: Arc<Context>, req: Request<Body>) -> anyhow::Result<Response<Body>> {
    let path = &req.uri().path().to_owned();
    if path == "/" {
        return Ok(Response::builder()
            .status(302)
            .header("Location", "https://git.nea.moe/nea/ursa-minor")
            .body(Body::empty())?);
    }
    // TODO: require authentication for these paths
    if let Some(meta_path) = path.strip_prefix("/_meta/") {
        return respond_to_meta(arc, req, meta_path).await;
    }
    if let Some(hypixel_path) = path.strip_prefix("/v1/hypixel/") {
        if let Some(resp) = hypixel::respond_to(arc, hypixel_path).await? {
            return Ok(resp);
        }
    }
    return make_error(404, format!("Unknown request path {}", path).as_str());
}

async fn wrap_error(arc: Arc<Context>, req: Request<Body>) -> anyhow::Result<Response<Body>> {
    return match respond_to(arc, req).await {
        Ok(x) => Ok(x),
        Err(e) => {
            let error_id = uuid::Uuid::new_v4();
            eprint!("Error id: {} {:?}", error_id, e);
            Ok(Response::builder()
                .status(500)
                .body(format!("500 Internal Error\n\nError id: {}\nI'm legally not allowed to give you more information", error_id).into())?)
        }
    };
}

fn config_var(name: &str) -> anyhow::Result<String> {
    env::var(format!("URSA_{}", name)).with_context(|| format!("Could not find {} expected to be found in the environment at URSA_{}", name, name))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if let Ok(path) = dotenv::dotenv() {
        println!("Loaded dotenv from {}", path.to_str().unwrap_or("?"));
    }
    let hypixel_token = config_var("HYPIXEL_TOKEN")?;
    let port = config_var("PORT")?.parse::<u16>().with_context(|| "Could not parse port at URSA_PORT")?;
    let https = HttpsConnector::new();
    let client = Client::builder()
        .build::<_, Body>(https);
    let arc = Arc::new(Context {
        client,
        hypixel_token,
        rules: vec![Rule {
            http_path: "player".into(),
            hypixel_path: "https://api.hypixel.net/player".into(),
            query_arguments: vec!["uuid".into()],
        }],
    });
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let service = make_service_fn(|_conn| {
        // TODO: replace arc with &'static
        let carc = Arc::clone(&arc);
        async move {
            Ok::<_, anyhow::Error>(service_fn(move |req| { wrap_error(Arc::clone(&carc), req) }))
        }
    });
    let server = Server::bind(&addr).serve(service);
    println!("Ursa minor rises above the sky!");
    println!("Now listening at {}", addr);
    let mut s = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        _ = s.recv() => {
            println!("Terminated! It's time to say goodbye.");
            Ok(())
        }
        it = tokio::signal::ctrl_c() => {
            it?;
            println!("Interrupted! It's time to say goodbye.");
            Ok(())
        }
        it = server =>{
            it?;
            Ok(())
        }
    }
}
