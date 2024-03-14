use hyper::{body::Buf, Body, Response};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{mojang::JWTPrincipal, util::MillisecondTimestamp, RequestContext};

pub async fn respond_to(
    context: RequestContext,
    path: &str,
    principal: JWTPrincipal,
) -> anyhow::Result<Option<Response<Body>>> {
    if path == "reportinventory" {
        return Ok(report_inventory(context, &principal).await.map(Some)?);
    }
    if path == "requestinventories" {
        return Ok(request_inventory().await.map(Some)?);
    }
    Ok(None)
}

async fn request_inventory() -> anyhow::Result<Response<Body>> {
    let mut content = vec![];
    let mut d = vec![];
    if tokio::fs::try_exists("reports").await.unwrap_or(false) {
        let mut files = tokio::fs::read_dir("reports").await?;
        while let Some(file) = files.next_entry().await? {
            let mut file = tokio::fs::File::open(file.path()).await?;
            file.read_to_end(&mut d).await?;
            let data = serde_json::from_slice::<Report>(&d)?;
            content.push(data);
        }
    }

    return Ok(Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(serde_json::to_string(&InventoryList { entries: content })?.into())?
        .into());
}

#[derive(Deserialize, Serialize)]
pub struct InventoryList {
    entries: Vec<Report>,
}

#[derive(Deserialize, Serialize)]
pub struct Slot {
    slot_index: i32,
    item: Option<String>,
}

#[derive(Deserialize, Serialize)]
pub struct Inventory {
    title: String,
    slots: Vec<Slot>,
}

#[derive(Deserialize, Serialize)]
pub struct Report {
    inventory: Inventory,
    reporter_uuid: uuid::Uuid,
    report_timestamp: MillisecondTimestamp,
    report_uuid: uuid::Uuid,
}

async fn report_inventory(
    context: RequestContext,
    principal: &JWTPrincipal,
) -> anyhow::Result<Response<Body>> {
    let buffer = hyper::body::aggregate(context.request).await?;
    let payload = serde_json::from_reader::<_, Inventory>(buffer.reader())?;
    let report = Report {
        inventory: payload,
        reporter_uuid: principal.id,
        report_timestamp: MillisecondTimestamp::now()?,
        report_uuid: uuid::Uuid::new_v4(),
    };
    let stringified = serde_json::to_vec(&report)?;
    tokio::fs::create_dir_all("reports").await?;
    let mut file = tokio::fs::File::create(format!("reports/{}.json", report.report_uuid)).await?;
    file.write_all(&stringified).await?;
    Ok(Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body("{\"message\": \"§aThank you for helping us help you help us all!\"}".into())?
        .into())
}
