use crate::global_application_config;
use crate::util::{MillisecondTimestamp, UrlForRequest};
use base64::Engine;
use futures::{AsyncReadExt, StreamExt};
use hyper::{Body, Method, Request, StatusCode};
use influxdb::InfluxDbWriteable;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_with::serde_as;
use simdnbt::owned::{BaseNbt, NbtCompound, NbtTag};
use std::collections::HashMap;
use std::io::Cursor;
use std::io::Read;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, span, warn, Level};
use url::Url;
use uuid::Uuid;

type A<T> = Arc<[T]>;
type S = Arc<str>;
#[derive(Deserialize, Serialize, Default, Debug)]
struct AuctionPage {
    #[serde(rename = "lastUpdated")]
    last_updated: Option<MillisecondTimestamp>,
    page: u32,
    #[serde(rename = "totalPages")]
    total_pages: u32,
    auctions: A<Auction>,
}

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

macro_rules! nbt_use {
    ($o:expr, $n:expr, $t:ident) => {
        match ::simdnbt::owned::NbtCompound::get($o, $n) {
            ::std::option::Option::Some(::simdnbt::owned::NbtTag::$t(it)) => {
                ::std::option::Option::Some(it)
            }
            _ => return ::std::option::Option::None,
        }
    };
}
struct ItemStack<'a>(&'a NbtCompound);

impl<'a> ItemStack<'a> {
    fn new(compound: &'a NbtCompound) -> Self {
        Self(compound)
    }
    pub fn extra_attributes(&self) -> Option<ExtraAttributes<'a>> {
        let tag = nbt_use!(self.0, "tag", Compound)?;
        let extra_attr = nbt_use!(tag, "ExtraAttributes", Compound)?;
        Some(ExtraAttributes(extra_attr))
    }
}
struct ExtraAttributes<'a>(&'a NbtCompound);

impl<'a> ExtraAttributes<'a> {
    pub fn id(&self) -> Option<S> {
        let str = nbt_use!(self.0, "id", String)?;
        Some(str.to_str().into())
    }
}

impl Auction {
    fn needs_processing(&self, last_full_scan: Option<MillisecondTimestamp>) -> bool {
        match (self.last_updated, last_full_scan) {
            (Some(auction), Some(scan)) => auction < scan,
            _ => true,
        }
    }

    #[tracing::instrument(skip_all)]
    fn item_bytes(&self) -> anyhow::Result<A<u8>> {
        let base64_decoded = base64::engine::general_purpose::STANDARD.decode(
            self.item_bytes_compressed
                .as_ref()
                .as_bytes(),
        )?;

        Ok(base64_decoded.into())
    }
    #[tracing::instrument(skip_all)]
    pub async fn raw_nbt(&self) -> anyhow::Result<BaseNbt> {
        let mut ungzipped = Vec::new();
        let input = self.item_bytes()?;
        let mut decoder = flate2::read::GzDecoder::new(input.as_ref());
        decoder.read_to_end(&mut ungzipped)?;
        let mut c: Cursor<&[u8]> = Cursor::new(ungzipped.as_slice());
        let tag = simdnbt::owned::read(&mut c)?;
        Ok(tag.unwrap())
    }
    async fn item_stack(&self) -> anyhow::Result<NbtCompound> {
        let nbt = self.raw_nbt().await?;
        match nbt.as_compound().take("i") {
            None => anyhow::bail!("Missing root i tag"),
            Some(NbtTag::List(list)) => {
                let tag = list
                    .into_compounds()
                    .ok_or(anyhow::anyhow!("Expected compound tag"))?
                    .swap_remove(0);
                Ok(tag)
            }
            _ => {
                // TODO: 'a borrow a lot of things
                anyhow::bail!("Misshapen root tag");
            }
        }
    }
}

#[tracing::instrument]
async fn request_ah_page(page_number: u32) -> anyhow::Result<AuctionPage> {
    let args = [("page", format!("{page_number}"))];
    let url = Url::parse_with_params("https://api.hypixel.net/v2/skyblock/auctions", args)?;
    let request = Request::builder()
        .url(url)?
        .method(Method::GET)
        // .header("API-Key", &global_application_config.hypixel_token.0)
        .body(Body::empty())?;
    let response = global_application_config.client.request(request).await?;
    if response.status() == StatusCode::NOT_FOUND {
        return Ok(AuctionPage::default());
    }
    let buffer = hyper::body::to_bytes(response.into_body()).await?;
    let page: AuctionPage = serde_json::from_slice(&buffer)?;
    Ok(page)
}
/// Returns the timestamp that this update was processed
#[tracing::instrument]
async fn item_ah_scan_fallible(
    // TODO: inherit cancellation token
    last_full_scan: Option<MillisecondTimestamp>,
) -> anyhow::Result<MillisecondTimestamp> {
    // Also request https://api.hypixel.net/v2/skyblock/auctions_ended
    // For ended auctions
    let initial_page = request_ah_page(0).await?;

    let mut all_prices: Vec<(A<S>, f64)> = vec![];
    all_prices.extend(
        process_page(&initial_page, last_full_scan)
            .await?
            .into_iter(),
    );
    let pages =
        futures::stream::iter((1..initial_page.total_pages).map(|page| request_ah_page(page)))
            .buffer_unordered(8)
            .collect::<Vec<_>>()
            .await;
    info!("Web requests completed");
    for page in pages {
        let page = page?;
        all_prices.extend(process_page(&page, last_full_scan).await?.into_iter());
    }
    info!("Prices aggregated.");

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
    info!("Prices updated in influx: {res}");
    Ok(())
}

#[tracing::instrument(skip_all)]
async fn process_page(
    page: &AuctionPage,
    last_full_scan: Option<MillisecondTimestamp>,
) -> anyhow::Result<Vec<(A<S>, f64)>> {
    let mut v = vec![];
    for auction in &*page.auctions {
        if !auction.needs_processing(last_full_scan) {
            continue;
        }
        match auction.item_stack().await {
            Ok(item_stack) => {
                let bucket = find_buckets(&ItemStack::new(&item_stack));
                if auction.bin {
                    v.push((bucket, auction.starting_bid));
                } else {
                    // Eternal death upon auctions (at least until i get around to parsing recently ended auctions)
                }
            }
            Err(err) => {
                error!(
                    %err,
                    "Could not parse item with auction id {}: {:?}",
                    auction.uuid,
                    auction.raw_nbt().await
                );
            }
        }
    }
    Ok(v)
}

fn find_buckets(stack: &ItemStack) -> A<S> {
    let Some(attr) = &stack.extra_attributes() else {
        return [].into();
    };
    let Some(id) = &attr.id() else {
        return [].into();
    };
    let mut ids: Vec<S> = vec![];
    ids.push(id.clone());
    ids.into()
}

async fn item_ah_scan(last_full_scan: &mut Option<MillisecondTimestamp>) -> Duration {
    match item_ah_scan_fallible(*last_full_scan).await {
        Ok(timestamp) => {
            *last_full_scan = Some(timestamp);
            let d = Duration::from_secs(70); // 60 seconds update interval + 10 seconds lenience
            let w = timestamp + d;
            let c = w.wait_time_or_zero();
            debug!(
                "Calculated wait. Ts: {timestamp:?} + {d:?} = {w:?}. Wait: {c:?}. Now: {:?}",
                MillisecondTimestamp::now()
            );
            c
        }
        Err(er) => {
            error!(%er, "Encountered error during scanning",);
            Duration::from_secs(30)
        }
    }
}

#[tracing::instrument(skip_all)]
async fn loop_body(cancellation_token: CancellationToken) {
    info!("Auction house collection loop started.");
    debug!("Debug logging is enabled.");
    let mut wait_time = Duration::ZERO;
    let mut last_full_scan = None;
    loop {
        tokio::select! {
            _ = cancellation_token.cancelled() => {
                info!("Exiting ah loop");
                return
            }
            _ = tokio::time::sleep(wait_time) => {
                info!("Waited {wait_time:?} for next loop")
            }
        }
        tokio::select! {
            _ = cancellation_token.cancelled() => {
                warn!("Exiting ah loop during eval");
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
