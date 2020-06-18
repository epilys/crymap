//-
// Copyright (c) 2020, Jason Lingle
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

use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use log::{error, warn};

use super::defs::*;
use crate::account::mailbox_state::*;
use crate::account::model::*;
use crate::support::error::Error;
use crate::support::file_ops::IgnoreKinds;

/// The maximum number of rollup files that can exist before we start deleting
/// them (but not the transactions they contain) with a shorter grace period to
/// avoid filling up disk.
const EXCESS_ROLLUP_THRESHOLD: usize = 4;

/// Rollups other than the most recent which are older than this age are
/// candidates for deletion, including any transactions they contain.
#[cfg(not(test))]
const OLD_ROLLUP_GRACE_PERIOD: Duration = Duration::from_secs(24 * 3600);
/// Rollups other than the `EXCESS_ROLLUP_THRESHOLD` most recent rollups which
/// are older than this age are candidates for deletion, but not including any
/// transactions they contain.
#[cfg(not(test))]
const EXCESS_ROLLUP_GRACE_PERIOD: Duration = Duration::from_secs(60);

#[cfg(test)]
const OLD_ROLLUP_GRACE_PERIOD: Duration = Duration::from_secs(2);
#[cfg(test)]
const EXCESS_ROLLUP_GRACE_PERIOD: Duration = Duration::from_secs(1);

impl StatelessMailbox {
    /// Bring this mailbox into stateful mode.
    ///
    /// This corresponds to `SELECT`, `EXAMINE`, and `STATUS`.
    ///
    /// `QRESYNC` is performed with a separate call after selection.
    pub fn select(self) -> Result<(StatefulMailbox, SelectResponse), Error> {
        StatefulMailbox::select(self)
    }
}

impl StatefulMailbox {
    fn select(s: StatelessMailbox) -> Result<(Self, SelectResponse), Error> {
        let mut rollups = Self::list_rollups(&s)?;
        let state = rollups
            .pop()
            .and_then(|r| match s.read_state_file::<MailboxState>(&r.path) {
                Ok(state) => Some(state),
                Err(e) => {
                    error!(
                        "{} Error reading {}, starting from empty state: {}",
                        s.log_prefix,
                        r.path.display(),
                        e
                    );
                    None
                }
            })
            .unwrap_or_else(MailboxState::new);

        let mut this = Self {
            recency_frontier: state.max_modseq().map(Modseq::uid),
            s,
            state,
            suggest_rollup: 0,
        };
        this.poll()?;

        if !this.s.read_only {
            let s_clone = this.s.clone();
            rayon::spawn(move || {
                if let Err(err) = s_clone.message_scheme().gc(
                    &s_clone.common_paths.tmp,
                    &s_clone.common_paths.garbage,
                    0,
                ) {
                    warn!(
                        "{} Error garbage collecting messages: {}",
                        s_clone.log_prefix, err
                    );
                    return;
                }

                // We can expunge all data transactions which are included in
                // the latest one with `delete_transactions` set --- we know
                // that all reasonable processes will be looking at that one or
                // something later and won't care about the old rollups.
                let expunge_before_cid = rollups
                    .iter()
                    .filter(|r| r.delete_transactions)
                    .map(|r| r.cid)
                    .max()
                    .unwrap_or(Cid(0));

                if let Err(err) = s_clone.change_scheme().gc(
                    &s_clone.common_paths.tmp,
                    &s_clone.common_paths.garbage,
                    expunge_before_cid.0,
                ) {
                    warn!(
                        "{} Error garbage collecting changes: {}",
                        s_clone.log_prefix, err
                    );
                } else {
                    for rollup in rollups {
                        if rollup.delete_rollup {
                            if let Err(err) =
                                fs::remove_file(&rollup.path).ignore_not_found()
                            {
                                warn!(
                                    "{} Error removing {}: {}",
                                    s_clone.log_prefix,
                                    rollup.path.display(),
                                    err
                                );
                            }
                        }
                    }
                }
            });
        }

        let select_response = SelectResponse {
            flags: this.state.flags().map(|(_, f)| f.to_owned()).collect(),
            exists: this.state.num_messages(),
            recent: this.count_recent(),
            unseen: this
                .state
                .seqnums_uids()
                .filter(|&(_, uid)| {
                    this.state
                        .flag_id(&Flag::Seen)
                        .map(|fid| !this.state.test_flag(fid, uid))
                        .unwrap_or(true)
                })
                .next()
                .map(|(s, _)| s),
            uidnext: this.state.next_uid().unwrap_or(Uid::MAX),
            uidvalidity: this.s.uid_validity()?,
            read_only: this.s.read_only,
            max_modseq: this.state.report_max_modseq(),
        };
        Ok((this, select_response))
    }

    fn list_rollups(s: &StatelessMailbox) -> Result<Vec<RollupInfo>, Error> {
        match fs::read_dir(s.root.join("rollup")) {
            Err(e) if io::ErrorKind::NotFound == e.kind() => Ok(vec![]),
            Err(e) => Err(e.into()),
            Ok(it) => {
                let mut ret = Vec::new();
                let now = SystemTime::now();

                for entry in it {
                    let entry = entry?;
                    let modseq = match entry
                        .file_name()
                        .to_str()
                        .and_then(|n| u64::from_str_radix(n, 10).ok())
                        .and_then(Modseq::of)
                    {
                        Some(ms) => ms,
                        // Ignore inscrutable filenames
                        None => continue,
                    };

                    let md = match entry.metadata() {
                        Ok(md) => md,
                        // NotFound => we lost a race with another process
                        // Ignore the now-deleted file and carry on
                        Err(e) if io::ErrorKind::NotFound == e.kind() => {
                            continue
                        }
                        Err(e) => return Err(e.into()),
                    };

                    ret.push(RollupInfo {
                        cid: modseq.cid(),
                        path: entry.path(),
                        age: md
                            .modified()
                            .ok()
                            .and_then(|modified| {
                                now.duration_since(modified).ok()
                            })
                            .unwrap_or(Duration::from_secs(0)),
                        delete_rollup: false,
                        delete_transactions: false,
                    });
                }

                classify_rollups(&mut ret);
                Ok(ret)
            }
        }
    }
}

/// Order `rollups` so that the "latest" (i.e., the one to load from) is at the
/// end, and `delete_rollup` and `delete_transactions` are set appropriately.
fn classify_rollups(rollups: &mut [RollupInfo]) {
    if rollups.is_empty() {
        return;
    }

    rollups.sort_unstable();

    let len = rollups.len();

    // Any rollup other than the one with the greatest CID which is older than
    // the "OLD" threshold can be deleted along with any transactions it
    // contains.
    for rollup in &mut rollups[..len - 1] {
        if rollup.age >= OLD_ROLLUP_GRACE_PERIOD {
            rollup.delete_rollup = true;
            rollup.delete_transactions = true;
        }
    }

    // If we're starting to accumulate too many rollups, get rid of the oldest
    // ones more aggressively, but leave the transactions around.
    if len > EXCESS_ROLLUP_THRESHOLD {
        for rollup in &mut rollups[..len - EXCESS_ROLLUP_THRESHOLD] {
            if rollup.age >= EXCESS_ROLLUP_GRACE_PERIOD {
                rollup.delete_rollup = true;
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct RollupInfo {
    // First field since it's the main thing we sort by
    // We only include the CID since we also use this to determine which CIDs
    // can be expunged during cleanup. While Modseqs /should/ be totally
    // ordered, this is a more conservative behaviour.
    cid: Cid,
    age: Duration,
    path: PathBuf,
    delete_rollup: bool,
    delete_transactions: bool,
}

#[cfg(test)]
mod test {
    use super::*;

    fn r(cid: u32, age_ms: u64) -> RollupInfo {
        RollupInfo {
            cid: Cid(cid),
            path: PathBuf::new(),
            age: Duration::from_millis(age_ms),
            delete_rollup: false,
            delete_transactions: false,
        }
    }

    #[test]
    fn classify_rollups_empty() {
        classify_rollups(&mut []);
    }

    #[test]
    fn classify_rollups_single_young() {
        let mut rollups = [r(1234, 100)];
        classify_rollups(&mut rollups);
        assert_eq!([r(1234, 100)], rollups);
    }

    #[test]
    fn classify_rollups_single_old() {
        let mut rollups = [r(1234, 10_000_000)];
        classify_rollups(&mut rollups);
        assert_eq!([r(1234, 10_000_000)], rollups);
    }

    #[test]
    fn classify_rollups_one_young_one_old() {
        let mut rollups = [r(1000, 100), r(900, 10_000_000)];
        classify_rollups(&mut rollups);
        assert_eq!(
            [
                RollupInfo {
                    delete_rollup: true,
                    delete_transactions: true,
                    ..r(900, 10_000_000)
                },
                r(1000, 100)
            ],
            rollups
        );
    }

    #[test]
    fn classify_rollups_one_old_one_young() {
        let mut rollups = [r(900, 10_000_000), r(1000, 100)];
        classify_rollups(&mut rollups);
        assert_eq!(
            [
                RollupInfo {
                    delete_rollup: true,
                    delete_transactions: true,
                    ..r(900, 10_000_000)
                },
                r(1000, 100)
            ],
            rollups
        );
    }

    #[test]
    fn classify_rollups_excess() {
        let mut rollups = [
            r(1, 5_000), // delete everything
            r(2, 1_900), // delete rollup only
            r(3, 1_800), // excess allowance
            r(4, 1_700), // excess allowance
            r(5, 1_600), // excess allowance
            r(6, 1_500), // most recent
        ];
        classify_rollups(&mut rollups);
        assert_eq!(
            [
                RollupInfo {
                    delete_rollup: true,
                    delete_transactions: true,
                    ..r(1, 5_000)
                },
                RollupInfo {
                    delete_rollup: true,
                    ..r(2, 1_900)
                },
                r(3, 1_800),
                r(4, 1_700),
                r(5, 1_600),
                r(6, 1_500),
            ],
            rollups
        );
    }
}