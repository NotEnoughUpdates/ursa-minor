use std::fmt::{Debug, Formatter};
use std::ops::{Deref, DerefMut};
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

pub struct Obscure<T, const L: &'static str = "..">(pub T);

impl<T, const L: &'static str> Debug for Obscure<T, L> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(L)
    }
}

impl<T, const L: &'static str> Deref for Obscure<T, L> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T, const L: &'static str> DerefMut for Obscure<T, L> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
