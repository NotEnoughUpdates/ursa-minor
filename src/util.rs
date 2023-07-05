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

pub struct Obscure<T>(pub T);

impl<T> Debug for Obscure<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("..")
    }
}

impl<T> Deref for Obscure<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for Obscure<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
