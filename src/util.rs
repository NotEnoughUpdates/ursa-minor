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

use hyper::http::request::Builder;
use hyper::Uri;
use serde::{Deserialize, Serialize};
use std::fmt::{Debug, Formatter};
use std::ops::{Add, Deref, DerefMut};
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use url::Url;

// Sadly need to use Url for url encoding, since hypers uri does not have that capability
pub trait UrlForRequest {
    fn url(self, url: Url) -> anyhow::Result<Self>
    where
        Self: Sized;
}

impl UrlForRequest for Builder {
    fn url(self, url: Url) -> anyhow::Result<Self> {
        Ok(self.uri(Uri::from_str(url.as_str())?))
    }
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

#[derive(Debug, Deserialize, Serialize, Clone, Copy, Ord, PartialOrd, Eq, PartialEq)]
pub struct MillisecondTimestamp(pub u64);

impl Add<Duration> for MillisecondTimestamp {
    type Output = MillisecondTimestamp;

    fn add(self, rhs: Duration) -> Self::Output {
        MillisecondTimestamp(self.0 + rhs.as_millis() as u64)
    }
}

impl TryFrom<SystemTime> for MillisecondTimestamp {
    type Error = anyhow::Error;

    fn try_from(value: SystemTime) -> anyhow::Result<Self, Self::Error> {
        // Fails in ~580 billion years
        Ok(MillisecondTimestamp(
            value.duration_since(UNIX_EPOCH)?.as_millis() as u64,
        ))
    }
}
