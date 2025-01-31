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

use super::super::defs::*;
use crate::account::model::Flag;
use crate::support::error::Error;
use crate::test_data::*;

#[test]
fn append_message() {
    let setup = set_up();
    let mut client = setup.connect("3501meam");
    quick_log_in(&mut client);
    quick_create(&mut client, "3501meam");

    client
        .start_append(
            "3501meam",
            s::AppendFragment::default(),
            ENRON_SMALL_MULTIPARTS[0],
        )
        .unwrap();
    let mut buffer = Vec::new();
    let mut responses = client.finish_append(&mut buffer).unwrap();
    assert!(responses.len() >= 1);
    assert_tagged_ok_any(responses.pop().unwrap());

    command!(mut responses = client, c("EXAMINE 3501meam"));
    assert_tagged_ok_any(responses.pop().unwrap());
    has_untagged_response_matching! {
        s::Response::Exists(1) in responses
    };

    client
        .start_append(
            "3501meam",
            s::AppendFragment::default(),
            ENRON_SMALL_MULTIPARTS[1],
        )
        .unwrap();

    let mut buffer = Vec::new();
    let mut responses = client.finish_append(&mut buffer).unwrap();
    assert!(responses.len() >= 1);
    assert_tagged_ok_any(responses.pop().unwrap());
    has_untagged_response_matching! {
        s::Response::Exists(2) in responses
    };
}

#[test]
fn append_with_new_flag() {
    let setup = set_up();
    let mut client = setup.connect("3501meaf");
    quick_log_in(&mut client);
    quick_create(&mut client, "3501meaf");
    quick_select(&mut client, "3501meaf");

    ok_command!(client, c("XCRY FLAGS ON"));

    client
        .start_append(
            "3501meaf",
            s::AppendFragment {
                flags: Some(vec![Flag::Keyword("3501meaf".to_owned())]),
                ..s::AppendFragment::default()
            },
            ENRON_SMALL_MULTIPARTS[0],
        )
        .unwrap();
    let mut buffer = Vec::new();
    let mut responses = client.finish_append(&mut buffer).unwrap();
    assert!(responses.len() >= 2);
    assert_tagged_ok_any(responses.pop().unwrap());
    has_untagged_response_matching! {
        s::Response::Flags(ref flags) in responses => {
            assert!(flags.contains(&Flag::Keyword("3501meaf".to_owned())));
        }
    };
}

#[test]
fn copy_messages() {
    let setup = set_up();
    let mut client = setup.connect("3501mecm");
    quick_log_in(&mut client);
    quick_create(&mut client, "3501mecm/src");
    quick_create(&mut client, "3501mecm/dst1");
    quick_create(&mut client, "3501mecm/dst2");

    let num_messages = 5;
    quick_append_enron(&mut client, "3501mecm/src", num_messages);

    ok_command!(client, c("SELECT 3501mecm/src"));
    ok_command!(client, c("COPY 3:* 3501mecm/dst1"));
    ok_command!(client, c("XVANQUISH 2:3"));

    ok_command!(client, c("UID COPY 1:4 3501mecm/dst2"));

    command!(responses = client, c("EXAMINE 3501mecm/dst1"));

    has_untagged_response_matching! {
        s::Response::Exists(n) in responses => {
            assert_eq!(num_messages as u32 - 2, n);
        }
    };

    command!(responses = client, c("EXAMINE 3501mecm/dst2"));
    has_untagged_response_matching! {
        s::Response::Exists(2) in responses
    };
}

#[test]
fn expunge_messages() {
    let setup = set_up();
    let mut client = setup.connect("3501mexm");
    quick_log_in(&mut client);
    quick_create(&mut client, "3501mexm");
    quick_append_enron(&mut client, "3501mexm", 5);
    quick_select(&mut client, "3501mexm");

    ok_command!(client, c("STORE 2 +FLAGS (\\Deleted)"));
    command!(mut responses = client, c("EXPUNGE"));
    assert_tagged_ok(responses.pop().unwrap());
    assert_eq!(1, responses.len());
    has_untagged_response_matching! {
        s::Response::Expunge(2) in responses
    };
}

#[test]
fn append_copy_nx_destination() {
    let setup = set_up();
    let mut client = setup.connect("3501meac");
    quick_log_in(&mut client);
    quick_create(&mut client, "3501meac/noselect/child");
    quick_append_enron(&mut client, "3501meac", 1);

    ok_command!(client, c("DELETE 3501meac/noselect"));

    ok_command!(client, c("EXAMINE 3501meac"));

    command!([response] = client, c("COPY 1 nonexistent"));
    assert_error_response(
        response,
        Some(s::RespTextCode::TryCreate(())),
        Error::NxMailbox,
    );

    command!([response] = client, c("COPY 1 ../foo"));
    assert_error_response(
        response,
        Some(s::RespTextCode::TryCreate(())),
        Error::NxMailbox,
    );

    command!([response] = client, c("COPY 1 3501meac/noselect"));
    assert_error_response(
        response,
        Some(s::RespTextCode::Nonexistent(())),
        Error::MailboxUnselectable,
    );

    client
        .write_raw(b"A1 APPEND nonexistent {3+}\r\nfoo\r\n")
        .unwrap();
    let mut buffer = Vec::new();
    let response = client.read_one_response(&mut buffer).unwrap();
    assert_error_response(
        response,
        Some(s::RespTextCode::TryCreate(())),
        Error::NxMailbox,
    );

    client
        .write_raw(b"A2 APPEND ../foo {3+}\r\nfoo\r\n")
        .unwrap();
    let mut buffer = Vec::new();
    let response = client.read_one_response(&mut buffer).unwrap();
    assert_error_response(
        response,
        Some(s::RespTextCode::TryCreate(())),
        Error::NxMailbox,
    );

    client
        .write_raw(b"A3 APPEND 3501meac/noselect {3+}\r\nfoo\r\n")
        .unwrap();
    let mut buffer = Vec::new();
    let response = client.read_one_response(&mut buffer).unwrap();
    assert_error_response(
        response,
        Some(s::RespTextCode::Nonexistent(())),
        Error::MailboxUnselectable,
    );
}

#[test]
fn copy_expunged() {
    let setup = set_up();
    let mut client = setup.connect("3501mecx");
    quick_log_in(&mut client);
    quick_create(&mut client, "3501mecx");
    quick_append_enron(&mut client, "3501mecx", 2);
    quick_select(&mut client, "3501mecx");

    let mut client2 = setup.connect("3501mecx");
    quick_log_in(&mut client2);
    quick_select(&mut client2, "3501mecx");

    ok_command!(client2, c("XVANQUISH 1:2"));

    command!(mut responses = client, c("COPY 1 3501mecx"));
    unpack_cond_response! {
        (Some(_), s::RespCondType::No, None, Some(_)) =
            responses.pop().unwrap() => ()
    };
}

#[test]
fn error_conditions() {
    let setup = set_up();
    let mut client = setup.connect("3501meec");
    quick_log_in(&mut client);

    ok_command!(client, c("EXAMINE INBOX"));

    command!([response] = client, c("EXPUNGE"));
    assert_error_response(
        response,
        Some(s::RespTextCode::Cannot(())),
        Error::MailboxReadOnly,
    );

    // The distinction between COPY 1 and COPY 2 on empty mailboxes isn't
    // important. It's a consequence of the fact that we treat the maximum
    // valid seqnum as 1 until much later for empty mailboxes, since `*` needs
    // to resolve to *something*.
    command!([response] = client, c("COPY 1 INBOX"));
    assert_error_response(
        response,
        Some(s::RespTextCode::Nonexistent(())),
        Error::NxMessage,
    );

    command!([response] = client, c("COPY 2 INBOX"));
    unpack_cond_response! {
        (Some(_), s::RespCondType::Bad,
         Some(s::RespTextCode::ClientBug(())), Some(_)) = response => ()
    };
}
