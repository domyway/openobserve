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

//! This module contains models that can be serialized and deserialized as JSON
//! for HTTP responses and requests.

pub mod action;
#[cfg(feature = "enterprise")]
pub mod ai;
pub mod alerts;
#[cfg(feature = "cloud")]
pub mod billings;
pub mod dashboards;
pub mod destinations;
pub mod folders;
pub mod pipelines;
pub mod reports;
