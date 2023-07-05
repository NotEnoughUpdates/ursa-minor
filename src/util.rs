use std::str::FromStr;
use hyper::http::request::Builder;
use hyper::Uri;
use url::Url;

// Sadly need to use Url for url encoding, since hypers uri does not have that capability
pub trait UrlForRequest {
    fn url(self, url: Url) -> anyhow::Result<Self> where Self: Sized;
}

impl UrlForRequest for Builder {
    fn url(self, url: Url) -> anyhow::Result<Self> {
        Ok(self.uri(Uri::from_str(url.as_str())?))
    }
}

pub fn pure_true() -> bool {
    true
}