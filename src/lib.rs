//-
// Copyright (c) 2020, 2023, Jason Lingle
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

#![allow(
    clippy::collapsible_if,
    clippy::module_inception,
    clippy::type_complexity,
    clippy::too_many_arguments,
    clippy::needless_range_loop,
    clippy::needless_borrowed_reference,
    clippy::precedence
)]
#![deny(clippy::pattern_type_mismatch)]

#[cfg(test)]
macro_rules! assert_matches {
    ($expected:pat, $actual:expr $(,)*) => {
        match $actual {
            $expected => (),
            unexpected => panic!(
                "Expected {} matches {}, got {:?}",
                stringify!($expected),
                stringify!($actual),
                unexpected
            ),
        }
    };
}

#[cfg(test)]
macro_rules! assert_matches {
    ($expected:pat, $actual:expr $(,)*) => {
        match $actual {
            $expected => (),
            unexpected => panic!(
                "Expected {} matches {}, got {:?}",
                stringify!($expected),
                stringify!($actual),
                unexpected
            ),
        }
    };
}

#[macro_use]
pub mod support;

pub mod account;
pub mod crypt;
pub mod imap;
pub mod mime;
pub mod smtp;

#[cfg(test)]
pub mod test_data;
