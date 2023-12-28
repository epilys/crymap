//-
// Copyright (c) 2023, Jason Lingle
//
// This file is part of Crymap.
//
// Crymap is free software: you can  redistribute it and/or modify it under the
// terms of  the GNU General Public  License as published by  the Free Software
// Foundation, either version  3 of the License, or (at  your option) any later
// version.
//
// Crymap is distributed  in the hope that  it will be useful,  but WITHOUT ANY
// WARRANTY; without  even the implied  warranty of MERCHANTABILITY  or FITNESS
// FOR  A PARTICULAR  PURPOSE.  See the  GNU General  Public  License for  more
// details.
//
// You should have received a copy of the GNU General Public License along with
// Crymap. If not, see <http://www.gnu.org/licenses/>.

#![allow(dead_code)] // TODO REMOVE

mod canonicalisation;
mod error;
mod hash;
mod header;

pub use canonicalisation::{
    BodyCanonicalisation, BodyCanonicaliser, Canonicalisation,
    HeaderCanonicalisation,
};
pub use error::*;
pub use header::{
    Algorithm, HashAlgorithm, Header, SignatureAlgorithm, TxtFlags, TxtRecord,
    HEADER_NAME,
};
