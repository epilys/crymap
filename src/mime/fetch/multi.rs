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

use std::mem;

use chrono::prelude::*;

use super::bodystructure;
use super::envelope;
use super::section;
use super::simple;
use crate::account::model::*;
use crate::mime::grovel::{Visitor, VisitorMap};
use crate::mime::header;
use crate::support::error::Error;

/// The types that can be fetched in parallel by `MultiFetcher`.
#[derive(Debug)]
pub enum FetchedItem {
    /// A placeholder for an item not yet fetched.
    ///
    /// This is not supposed to escape `MultiFetcher`.
    Nil,
    Uid(Uid),
    Modseq(Modseq),
    Flags(simple::FlagsInfo),
    Rfc822Size(u32),
    InternalDate(DateTime<FixedOffset>),
    SaveDate(Option<DateTime<FixedOffset>>),
    EmailId(String),
    ThreadIdNil,
    Envelope(Box<envelope::Envelope>),
    BodyStructure(Box<bodystructure::BodyStructure>),
    BodySection(
        (
            section::BodySection,
            Result<section::FetchedBodySection, Error>,
        ),
    ),
}

impl FetchedItem {
    pub fn into_envelope(self) -> Option<envelope::Envelope> {
        match self {
            FetchedItem::Envelope(e) => Some(*e),
            _ => None,
        }
    }

    pub fn into_body_structure(self) -> Option<bodystructure::BodyStructure> {
        match self {
            FetchedItem::BodyStructure(s) => Some(*s),
            _ => None,
        }
    }

    pub fn into_body_section(
        self,
    ) -> Option<(
        section::BodySection,
        Result<section::FetchedBodySection, Error>,
    )> {
        match self {
            FetchedItem::BodySection(s) => Some(s),
            _ => None,
        }
    }

    fn into_none<T>(self) -> Option<T> {
        None
    }
}

type Fetcher = Box<dyn Visitor<Output = FetchedItem>>;

/// Performs multiple fetch operations in parallel, i.e., by distributing the
/// parse events to multiple sub-fetchers (nothing to do with concurrency).
///
/// The fetch output is a `Vec<FetchedItem>` which is parallel to the input
/// fetcher list.
#[derive(Debug, Default)]
pub struct MultiFetcher {
    fetchers: Vec<Option<Fetcher>>,
    results: Vec<FetchedItem>,
    remaining: usize,
}

impl MultiFetcher {
    /// Create a new, empty `MultiFetcher`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Collect the UID in this fetch.
    pub fn add_uid(&mut self) {
        self.add_fetcher(Box::new(VisitorMap::new(
            Box::new(simple::UidFetcher),
            FetchedItem::Uid,
            FetchedItem::into_none,
        )))
    }

    /// Collect the `Modseq` in this fetch.
    pub fn add_modseq(&mut self) {
        self.add_fetcher(Box::new(VisitorMap::new(
            Box::new(simple::ModseqFetcher),
            FetchedItem::Modseq,
            FetchedItem::into_none,
        )))
    }

    /// Fetch the flags of the message.
    pub fn add_flags(&mut self) {
        self.add_fetcher(Box::new(VisitorMap::new(
            Box::new(simple::FlagsFetcher::new()),
            FetchedItem::Flags,
            FetchedItem::into_none,
        )))
    }

    /// Fetch the "RFC822 Size" of the message.
    pub fn add_rfc822size(&mut self) {
        self.add_fetcher(Box::new(VisitorMap::new(
            Box::new(simple::Rfc822SizeFetcher),
            FetchedItem::Rfc822Size,
            FetchedItem::into_none,
        )))
    }

    /// Fetch the "internal date" of the message.
    pub fn add_internal_date(&mut self) {
        self.add_fetcher(Box::new(VisitorMap::new(
            Box::new(simple::InternalDateFetcher),
            FetchedItem::InternalDate,
            FetchedItem::into_none,
        )))
    }

    /// Fetch the "save date" of the message.
    pub fn add_save_date(&mut self) {
        self.add_fetcher(Box::new(VisitorMap::new(
            Box::new(simple::SaveDateFetcher),
            FetchedItem::SaveDate,
            FetchedItem::into_none,
        )))
    }

    /// Fetch the email id of the message.
    pub fn add_email_id(&mut self) {
        self.add_fetcher(Box::new(VisitorMap::new(
            Box::new(simple::EmailIdFetcher),
            FetchedItem::EmailId,
            FetchedItem::into_none,
        )))
    }

    /// "Fetch" the token `ThreadIdNil` item.
    pub fn add_thread_id(&mut self) {
        self.add_fetcher(Box::new(VisitorMap::new(
            Box::new(simple::UidFetcher),
            |_| FetchedItem::ThreadIdNil,
            FetchedItem::into_none,
        )))
    }

    /// Add an `EnvelopeFetcher` as a sub-fetcher.
    pub fn add_envelope(&mut self) {
        self.add_fetcher(Box::new(VisitorMap::new(
            Box::new(envelope::EnvelopeFetcher::new()),
            |e| FetchedItem::Envelope(Box::new(e)),
            FetchedItem::into_envelope,
        )));
    }

    /// Add a `BodyStructureFetcher` as a sub-fetcher.
    pub fn add_body_structure(&mut self) {
        self.add_fetcher(Box::new(VisitorMap::new(
            Box::new(bodystructure::BodyStructureFetcher::new()),
            |bs| FetchedItem::BodyStructure(Box::new(bs)),
            FetchedItem::into_body_structure,
        )));
    }

    /// Add the given `BodySection` fetcher as a sub-fetcher.
    pub fn add_section(&mut self, section: section::Fetcher) {
        self.add_fetcher(Box::new(VisitorMap::new(
            section,
            FetchedItem::BodySection,
            FetchedItem::into_body_section,
        )));
    }

    fn add_fetcher(&mut self, fetcher: Fetcher) {
        self.fetchers.push(Some(fetcher));
        self.results.push(FetchedItem::Nil);
        self.remaining += 1;
    }

    fn on_fetchers(
        &mut self,
        mut f: impl FnMut(&mut Fetcher) -> Result<(), FetchedItem>,
    ) -> Result<(), Vec<FetchedItem>> {
        for i in 0..self.fetchers.len() {
            let result =
                self.fetchers[i].as_mut().map(&mut f).unwrap_or(Ok(()));
            if let Err(result) = result {
                self.results[i] = result;
                self.fetchers[i] = None;
                self.remaining -= 1;
            }
        }

        if 0 == self.remaining {
            Err(mem::take(&mut self.results))
        } else {
            Ok(())
        }
    }
}

impl Visitor for MultiFetcher {
    type Output = Vec<FetchedItem>;

    fn uid(&mut self, uid: Uid) -> Result<(), Self::Output> {
        self.on_fetchers(|fetcher| fetcher.uid(uid))
    }

    fn email_id(&mut self, id: &str) -> Result<(), Self::Output> {
        self.on_fetchers(|fetcher| fetcher.email_id(id))
    }

    fn last_modified(&mut self, modseq: Modseq) -> Result<(), Self::Output> {
        self.on_fetchers(|fetcher| fetcher.last_modified(modseq))
    }

    fn savedate(
        &mut self,
        savedate: DateTime<Utc>,
    ) -> Result<(), Self::Output> {
        self.on_fetchers(|fetcher| fetcher.savedate(savedate))
    }

    fn want_flags(&self) -> bool {
        self.fetchers
            .iter()
            .filter_map(Option::as_ref)
            .any(|fetcher| fetcher.want_flags())
    }

    fn flags(&mut self, flags: &[Flag]) -> Result<(), Self::Output> {
        self.on_fetchers(|fetcher| fetcher.flags(flags))
    }

    fn recent(&mut self) -> Result<(), Self::Output> {
        self.on_fetchers(|fetcher| fetcher.recent())
    }

    fn end_flags(&mut self) -> Result<(), Self::Output> {
        self.on_fetchers(|fetcher| fetcher.end_flags())
    }

    fn rfc822_size(&mut self, size: u32) -> Result<(), Self::Output> {
        self.on_fetchers(|fetcher| fetcher.rfc822_size(size))
    }

    fn metadata(
        &mut self,
        metadata: &MessageMetadata,
    ) -> Result<(), Self::Output> {
        self.on_fetchers(|fetcher| fetcher.metadata(metadata))
    }

    fn raw_line(&mut self, line: &[u8]) -> Result<(), Self::Output> {
        self.on_fetchers(|fetcher| fetcher.raw_line(line))
    }

    fn header(
        &mut self,
        raw: &[u8],
        name: &str,
        value: &[u8],
    ) -> Result<(), Self::Output> {
        self.on_fetchers(|fetcher| fetcher.header(raw, name, value))
    }

    fn content_type(
        &mut self,
        ct: &header::ContentType<'_>,
    ) -> Result<(), Self::Output> {
        self.on_fetchers(|fetcher| fetcher.content_type(ct))
    }

    fn leaf_section(
        &mut self,
    ) -> Option<Box<dyn Visitor<Output = Self::Output>>> {
        for fetcher in self.fetchers.iter_mut().flatten() {
            if let Some(new) = fetcher.leaf_section() {
                *fetcher = new;
            }
        }
        None
    }

    fn start_content(&mut self) -> Result<(), Self::Output> {
        self.on_fetchers(|fetcher| fetcher.start_content())
    }

    fn content(&mut self, data: &[u8]) -> Result<(), Self::Output> {
        self.on_fetchers(|fetcher| fetcher.content(data))
    }

    fn start_part(
        &mut self,
    ) -> Option<Box<dyn Visitor<Output = Self::Output>>> {
        let mut sub = Self::new();
        for fetcher in &mut self.fetchers {
            let sub_fetcher =
                fetcher.as_mut().and_then(|fetcher| fetcher.start_part());
            if sub_fetcher.is_some() {
                sub.remaining += 1;
            }
            sub.fetchers.push(sub_fetcher);
            sub.results.push(FetchedItem::Nil);
        }

        if sub.remaining > 0 {
            Some(Box::new(sub))
        } else {
            None
        }
    }

    fn child_result(
        &mut self,
        mut child_result: Self::Output,
    ) -> Result<(), Self::Output> {
        for i in 0..self.fetchers.len() {
            let result = if let Some(fetcher) = self.fetchers[i].as_mut() {
                fetcher.child_result(mem::replace(
                    &mut child_result[i],
                    FetchedItem::Nil,
                ))
            } else {
                Ok(())
            };

            if let Err(result) = result {
                self.results[i] = result;
                self.fetchers[i] = None;
                self.remaining -= 1;
            }
        }

        if 0 == self.remaining {
            Err(mem::take(&mut self.results))
        } else {
            Ok(())
        }
    }

    fn end(&mut self) -> Self::Output {
        self.on_fetchers(|fetcher| Err(fetcher.end()))
            .expect_err("Failed to complete MultiFetcher.end()")
    }

    fn visit_default(&mut self) -> Result<(), Self::Output> {
        panic!("missing method on MultiFetcher")
    }
}

#[cfg(test)]
mod test {
    use std::io::Read;
    use std::sync::Arc;

    use super::*;
    use crate::mime::grovel;
    use crate::support::chronox::*;

    #[test]
    fn test_multi_fetch() {
        let common_paths = Arc::new(CommonPaths {
            tmp: std::env::temp_dir(),
            garbage: std::env::temp_dir(),
        });

        let mut fetcher = MultiFetcher::new();
        fetcher.add_envelope();
        fetcher.add_section(
            section::BodySection {
                subscripts: vec![3, 1],
                leaf_type: section::LeafType::Content,
                ..section::BodySection::default()
            }
            .fetcher(Arc::clone(&common_paths)),
        );
        fetcher.add_section(
            section::BodySection {
                subscripts: vec![2],
                leaf_type: section::LeafType::Mime,
                ..section::BodySection::default()
            }
            .fetcher(Arc::clone(&common_paths)),
        );
        fetcher.add_section(
            section::BodySection {
                subscripts: vec![4, 2, 2, 1],
                leaf_type: section::LeafType::Content,
                ..section::BodySection::default()
            }
            .fetcher(Arc::clone(&common_paths)),
        );
        fetcher.add_uid();
        fetcher.add_modseq();
        fetcher.add_flags();
        fetcher.add_rfc822size();
        fetcher.add_internal_date();
        fetcher.add_save_date();
        fetcher.add_email_id();

        let uid = Uid::u(42);
        let modseq = Modseq::of(56100);
        let internal_date =
            FixedOffset::zero().timestamp_millis_opt(1000).unwrap();
        let savedate = Utc.timestamp_millis_opt(2000).unwrap();
        let mut result = grovel::grovel(
            &mut grovel::SimpleAccessor {
                data: crate::test_data::RFC3501_P56.to_owned().into(),
                uid,
                email_id: Some("E1234".to_owned()),
                last_modified: modseq,
                recent: true,
                flags: vec![Flag::Deleted],
                rfc822_size: Some(1234),
                savedate: Some(savedate),
                metadata: MessageMetadata {
                    size: 1234,
                    internal_date,
                    email_id: [
                        1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
                    ],
                },
            },
            fetcher,
        )
        .unwrap();

        assert_eq!(11, result.len());

        match &result[0] {
            &FetchedItem::Envelope(ref envelope) => {
                assert_eq!("RFC 3501", envelope.subject.as_ref().unwrap());
            },
            r => panic!("Unexpected envelope result: {:#?}", r),
        }

        match &mut result[1] {
            &mut FetchedItem::BodySection((_, Ok(ref mut bs))) => {
                let mut content = String::new();
                bs.buffer.read_to_string(&mut content).unwrap();
                assert_eq!("Part 3.1\r\n", content);
            },
            r => panic!("Unexpected [3.1] result: {:#?}", r),
        }

        match &mut result[2] {
            &mut FetchedItem::BodySection((_, Ok(ref mut bs))) => {
                let mut content = String::new();
                bs.buffer.read_to_string(&mut content).unwrap();
                assert!(content.starts_with("Content-Id: 2"));
            },
            r => panic!("Unexpected [2] result: {:#?}", r),
        }

        match &mut result[3] {
            &mut FetchedItem::BodySection((_, Ok(ref mut bs))) => {
                let mut content = String::new();
                bs.buffer.read_to_string(&mut content).unwrap();
                assert_eq!("Part 4.2.2.1\r\n", content);
            },
            r => panic!("Unexpected [4.2.2.1] result: {:#?}", r),
        }

        match &result[4] {
            &FetchedItem::Uid(u) => assert_eq!(uid, u),
            r => panic!("Unexpected UID result: {:#?}", r),
        }

        match &result[5] {
            &FetchedItem::Modseq(m) => assert_eq!(modseq, m),
            r => panic!("Unexpected Modseq result: {:#?}", r),
        }

        match &result[6] {
            &FetchedItem::Flags(ref f) => {
                assert!(f.recent);
                assert_eq!(vec![Flag::Deleted], f.flags);
            },
            r => panic!("Unexpected Flags result: {:#?}", r),
        }

        match &result[7] {
            &FetchedItem::Rfc822Size(s) => assert_eq!(1234, s),
            r => panic!("Unexpected Rfc822Size result: {:#?}", r),
        }

        match &result[8] {
            &FetchedItem::InternalDate(id) => {
                assert_eq!(
                    FixedOffset::zero().timestamp_millis_opt(1000).unwrap(),
                    id
                )
            },
            r => panic!("Unexpected internal date result: {:#?}", r),
        }

        match &result[9] {
            &FetchedItem::SaveDate(sd) => {
                assert_eq!(Some(DateTime::<FixedOffset>::from(savedate)), sd);
            },
            r => panic!("Unexpected save date result: {r:#?}"),
        }

        match &result[10] {
            &FetchedItem::EmailId(ref id) => {
                assert_eq!("E1234", id);
            },
            r => panic!("Unexpected email id result: {:#?}", r),
        }
    }
}
