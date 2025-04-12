use crate::global_application_config;
use crate::util::{MillisecondTimestamp, UrlForRequest};
use base64::Engine;
use futures::StreamExt;
use hyper::body::Buf;
use hyper::{Body, Method, Request, StatusCode};
use influxdb::InfluxDbWriteable;
use nbt::Blob;
use serde::{Deserialize, Serialize};
use serde_with::base64::Base64;
use serde_with::serde_as;
use std::cmp::min;
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use url::Url;
use uuid::Uuid;

type A<T> = Arc<[T]>;
type S = Arc<str>;
#[serde_as]
#[derive(Deserialize, Serialize, Debug)]
struct Auction {
    uuid: Uuid,
    auctioneer: Uuid,
    profile_id: Uuid,
    coop: A<Uuid>,
    start: MillisecondTimestamp,
    end: MillisecondTimestamp,
    item_name: S,
    item_lore: S,
    extra: S,
    categories: A<S>,
    tier: S,
    starting_bid: f64,
    #[serde(rename = "item_bytes")]
    item_bytes_compressed: S,
    claimed: bool,
    highest_bid_amount: f64,
    #[serde(default)]
    last_updated: Option<MillisecondTimestamp>,
    bin: bool,
    item_uuid: Option<Uuid>,
    category: S,
}

#[derive(Deserialize, Serialize, Default, Debug)]
struct AuctionPage {
    #[serde(rename = "lastUpdated")]
    last_updated: Option<MillisecondTimestamp>,
    page: u32,
    #[serde(rename = "totalPages")]
    total_pages: u32,
    auctions: A<Auction>,
}

#[derive(Serialize, Deserialize)]
struct ItemHolder {
    i: Vec<ItemStack>,
}
const fn pure1() -> u8 {
    1
}
#[derive(Serialize, Deserialize, Debug)]
struct ItemStack {
    #[serde(rename = "Damage")]
    damage: u16,
    id: u16,
    #[serde(rename = "Count")]
    count: u8,
    tag: Tag,
}

#[derive(Serialize, Deserialize, Debug)]
struct Tag {
    #[serde(rename = "ExtraAttributes", default)]
    extra_attributes: ExtraAttributes,
}
#[derive(Serialize, Deserialize, Debug, Default)]
struct ExtraAttributes {
    id: Option<S>,
    uuid: Option<Uuid>,
}

impl Auction {
    fn needs_processing(&self, last_full_scan: Option<MillisecondTimestamp>) -> bool {
        match (self.last_updated, last_full_scan) {
            (Some(auction), Some(scan)) => auction < scan,
            _ => true,
        }
    }

    fn item_bytes(&self) -> anyhow::Result<Vec<u8>> {
        Ok(base64::engine::general_purpose::STANDARD
            .decode(self.item_bytes_compressed.as_ref().as_bytes())?)
    }
    fn item_stack(&self) -> anyhow::Result<ItemStack> {
        // TODO: from_reader is sometimes quite slow. maybe parse into a unzipped byte array first and then decode via nbt::de::from_reader directly.
        let mut nbt: ItemHolder = nbt::de::from_gzip_reader(Cursor::new(self.item_bytes()?))?;
        Ok(nbt.i.pop().ok_or_else(|| anyhow::anyhow!(""))?)
    }
    fn debug_nbt(&self) -> anyhow::Result<nbt::Blob> {
        let nbt: nbt::Blob = nbt::de::from_gzip_reader(Cursor::new(self.item_bytes()?))?;
        Ok(nbt)
    }
}

async fn request_ah_page(page_number: u32) -> anyhow::Result<AuctionPage> {
    let args = [("page", format!("{page_number}"))];
    let url = Url::parse_with_params("https://api.hypixel.net/v2/skyblock/auctions", args)?;
    let request = Request::builder()
        .url(url)?
        .method(Method::GET)
        // .header("API-Key", &global_application_config.hypixel_token.0)
        .body(Body::empty())?;
    println!("Requesting page {page_number}");
    let t = Instant::now();
    let response = global_application_config.client.request(request).await?;
    if response.status() == StatusCode::NOT_FOUND {
        return Ok(AuctionPage::default());
    }
    let buffer = hyper::body::to_bytes(response.into_body()).await?;
    println!(
        "Request for page {page_number} took {} seconds.",
        (Instant::now() - t).as_secs_f32()
    );
    let t = Instant::now();
    let page: AuctionPage = serde_json::from_slice(&buffer)?;
    println!(
        "JSON Parse for page {page_number} took {} seconds.",
        (Instant::now() - t).as_secs_f32()
    );
    Ok(page)
}
/// Returns the timestamp that this update was processed
async fn item_ah_scan_fallible(
    // TODO: inherit cancellation token
    last_full_scan: Option<MillisecondTimestamp>,
) -> anyhow::Result<MillisecondTimestamp> {
    // Also request https://api.hypixel.net/v2/skyblock/auctions_ended
    // For ended auctions
    let initial_page = request_ah_page(0).await?;

    let mut all_prices: Vec<(A<S>, f64)> = vec![];
    all_prices.extend(process_page(&initial_page, last_full_scan)?.into_iter());
    let pages =
        futures::stream::iter((1..initial_page.total_pages).map(|page| request_ah_page(page)))
            .buffer_unordered(8)
            .collect::<Vec<_>>()
            .await;
    println!("Web requests completed");
    for page in pages {
        let page = page?;
        all_prices.extend(process_page(&page, last_full_scan)?.into_iter());
    }
    println!("Prices aggregated.");

    update_prices(&all_prices).await?;
    Ok(initial_page
        .last_updated
        .ok_or(anyhow::anyhow!("initial page does not have a lastUpdated"))?)
}

#[derive(InfluxDbWriteable)]
struct PricePoint {
    time: MillisecondTimestamp,
    price: f64,
    #[influxdb(tag)]
    id: String, // TODO: ref this
}

async fn update_prices(all_prices: &[(impl AsRef<[S]>, f64)]) -> anyhow::Result<()> {
    let mut prices = HashMap::<S, _>::new();
    for (buckets, price) in all_prices {
        for bucket in buckets.as_ref().iter() {
            let original = prices.entry(bucket.clone()).or_insert(*price);
            *original = original.min(*price)
        }
    }
    println!("Prices bucketed");
    let ts = MillisecondTimestamp::now()?;
    let influx = influxdb::Client::new(&global_application_config.influx_url, "prices");
    let readings: Vec<_> = prices
        .into_iter()
        .map(|(k, v)| {
            PricePoint {
                time: ts,
                price: v,
                id: (*k).to_owned(),
            }
            .into_query("lowest_bin")
        })
        .collect();
    let res = influx.query(readings).await?;
    println!("Prices updated: {res}");
    Ok(())
}

fn process_page(
    page: &AuctionPage,
    last_full_scan: Option<MillisecondTimestamp>,
) -> anyhow::Result<Vec<(A<S>, f64)>> {
    let mut v = vec![];
    for auction in &*page.auctions {
        if !auction.needs_processing(last_full_scan) {
            continue;
        }
        match auction.item_stack() {
            Ok(item_stack) => {
                let bucket = find_buckets(&item_stack);
                if auction.bin {
                    v.push((bucket, auction.starting_bid));
                } else {
                    // Eternal death upon auctions (at least until i get around to parsing recently ended auctions)
                }
            }
            Err(err) => {
                eprintln!(
                    "Could not parse item with auction id {}: {:#}; {:?}",
                    auction.uuid,
                    err,
                    auction.debug_nbt().ok()
                );
            }
        }
    }
    Ok(v)
}

fn find_buckets(stack: &ItemStack) -> A<S> {
    let attr = &stack.tag.extra_attributes;
    let Some(id) = &attr.id else { return [].into() };
    let mut ids: Vec<S> = vec![];
    ids.push(id.clone());
    ids.into()
}

async fn item_ah_scan(last_full_scan: &mut Option<MillisecondTimestamp>) -> Duration {
    match item_ah_scan_fallible(*last_full_scan).await {
        Ok(timestamp) => {
            *last_full_scan = Some(timestamp);
            let d = Duration::from_secs(65);
            let w = (timestamp + d);
            let c = w.wait_time_or_zero();
            println!("Calculated wait. Ts: {timestamp:?} + {d:?} = {w:?}. Wait: {c:?}. Now: {:?}", MillisecondTimestamp::now());
            c
        }
        Err(er) => {
            eprintln!("Encountered error during scanning {er:#}");
            Duration::from_secs(30)
        }
    }
}

async fn loop_body(cancellation_token: CancellationToken) {
    println!("Auction house collection loop started.");
    let mut wait_time = Duration::ZERO;
    let mut last_full_scan = None;
    loop {
        tokio::select! {
            _ = cancellation_token.cancelled() => {
                println!("Exiting ah loop");
                return
            }
            _ = tokio::time::sleep(wait_time) => {
            }
        }
        tokio::select! {
            _ = cancellation_token.cancelled() => {
                println!("Exiting ah loop during eval");
                return
            }
              it = item_ah_scan(&mut last_full_scan) => {
                wait_time = it
            }
        }
    }
}

pub(crate) fn start_loop(cancellation_token: &CancellationToken) -> JoinHandle<()> {
    let token = cancellation_token.clone();
    tokio::spawn(async move {
        loop_body(token).await;
    })
}
