// Copyright 2025 OpenObserve Inc.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <http://www.gnu.org/licenses/>.

// #![feature(async_drop)]
#![feature(btree_cursors)]
#![feature(btree_extract_if)]

#[cfg(feature = "enterprise")]
pub mod cipher;
pub mod cli;
pub mod common;
pub mod handler;
pub mod job;
pub mod migration;
pub mod router;
pub mod service;

#[cfg(feature = "enterprise")]
pub mod super_cluster_queue;

pub(crate) static USER_AGENT_REGEX_FILE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/ua_regex/regexes.yaml"
));
