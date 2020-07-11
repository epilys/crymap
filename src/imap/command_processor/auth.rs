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

use std::borrow::Cow;
use std::fs;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::sync::Arc;

use log::{error, info, warn};

use super::defs::*;
use crate::account::account::{account_config_file, Account};
use crate::crypt::master_key::MasterKey;
use crate::support::{safe_name::is_safe_name, user_config::UserConfig};

impl CommandProcessor {
    pub(crate) fn cmd_log_in(
        &mut self,
        cmd: s::LogInCommand<'_>,
        _sender: SendResponse<'_>,
    ) -> CmdResult {
        if self.account.is_some() {
            return Err(s::Response::Cond(s::CondResponse {
                cond: s::RespCondType::Bad,
                code: None,
                quip: Some(Cow::Borrowed("Already logged in")),
            }));
        }

        if !is_safe_name(&cmd.userid) {
            return Err(s::Response::Cond(s::CondResponse {
                cond: s::RespCondType::No,
                code: None,
                quip: Some(Cow::Borrowed("Illegal user id")),
            }));
        }

        let mut user_dir = self.data_root.join(&*cmd.userid);
        let user_data_file = account_config_file(&user_dir);
        let (user_config, master_key) = fs::File::open(&user_data_file)
            .ok()
            .and_then(|f| {
                let mut buf = Vec::<u8>::new();
                f.take(65536).read_to_end(&mut buf).ok()?;
                toml::from_slice::<UserConfig>(&buf).ok()
            })
            .and_then(|config| {
                let master_key = MasterKey::from_config(
                    &config.master_key,
                    cmd.password.as_bytes(),
                )?;
                Some((config, master_key))
            })
            .ok_or_else(|| {
                s::Response::Cond(s::CondResponse {
                    cond: s::RespCondType::No,
                    code: None,
                    quip: Some(Cow::Borrowed("Bad user id or password")),
                })
            })?;

        // Login successful (at least barring further operational issues)

        self.log_prefix.push_str(":~");
        self.log_prefix.push_str(&cmd.userid);
        info!("{} Login successful", self.log_prefix);

        self.drop_privelages(&mut user_dir)?;

        let account = Account::new(
            self.log_prefix.clone(),
            user_dir,
            Some(Arc::new(master_key)),
        );
        account
            .init(&user_config.key_store)
            .map_err(map_error!(self))?;

        self.account = Some(account);
        Ok(s::Response::Cond(s::CondResponse {
            cond: s::RespCondType::Ok,
            code: Some(s::RespTextCode::Capability(
                super::commands::capability_data(),
            )),
            quip: Some(Cow::Borrowed("User login successful")),
        }))
    }

    fn drop_privelages(&mut self, user_dir: &mut PathBuf) -> PartialResult<()> {
        // Nothing to do if we aren't root
        if nix::unistd::ROOT != nix::unistd::getuid() {
            return Ok(());
        }

        // Before we can chroot, we need to figure out what our groups will be
        // once we drop down to the user, because we won't have access to
        // /etc/group after the chroot
        let md = match user_dir.metadata() {
            Ok(md) => md,
            Err(e) => {
                error!(
                    "{} Failed to stat '{}': {}",
                    self.log_prefix,
                    user_dir.display(),
                    e
                );
                return auth_misconfiguration();
            }
        };
        let target_uid =
            nix::unistd::Uid::from_raw(md.uid() as nix::libc::uid_t);
        let (has_user_groups, target_gid) = match nix::unistd::User::from_uid(
            target_uid,
        ) {
            Ok(Some(user)) => {
                match nix::unistd::initgroups(
                    &std::ffi::CString::new(user.name.to_owned())
                        .expect("Got UNIX user name with NUL?"),
                    user.gid,
                ) {
                    Ok(()) => (true, user.gid),
                    Err(e) => {
                        warn!(
                            "{} Failed to init groups for user: {}",
                            self.log_prefix, e
                        );
                        (false, user.gid)
                    }
                }
            }
            Ok(None) => {
                // Failure to access /etc/group is expected if we chroot'ed
                // into the system data directory already
                if !self.system_config.security.chroot_system {
                    warn!(
                        "{} No passwd entry for UID {}, assuming GID {}",
                        self.log_prefix,
                        target_uid,
                        md.gid()
                    );
                }
                (
                    false,
                    nix::unistd::Gid::from_raw(md.gid() as nix::libc::gid_t),
                )
            }
            Err(e) => {
                // Failure to access /etc/group is expected if we chroot'ed
                // into the system data directory already
                if !self.system_config.security.chroot_system {
                    warn!(
                        "{} Failed to look up passwd entry for UID {}, \
                         assuming GID {}: {}",
                        self.log_prefix,
                        target_uid,
                        md.gid(),
                        e
                    );
                }
                (
                    false,
                    nix::unistd::Gid::from_raw(md.gid() as nix::libc::gid_t),
                )
            }
        };

        if let Err(e) = nix::unistd::chdir(user_dir)
            .and_then(|()| nix::unistd::chroot(user_dir))
        {
            error!(
                "{} Chroot (forced because Crymap is running as root) \
                    into '{}' failed: {}",
                self.log_prefix,
                user_dir.display(),
                e
            );
            return auth_misconfiguration();
        }

        // Chroot successful, adjust the log prefix and path to reflect that
        self.log_prefix
            .push_str(&format!(":[chroot {}]", user_dir.display()));
        user_dir.push("/"); // Clears everything but '/'

        // Now we can finish dropping privileges
        if let Err(e) = if has_user_groups {
            Ok(())
        } else {
            nix::unistd::setgroups(&[target_gid])
        }
        .and_then(|()| nix::unistd::setgid(target_gid))
        .and_then(|()| nix::unistd::setuid(target_uid))
        {
            error!(
                "{} Failed to drop privileges to {}:{}: {}",
                self.log_prefix, target_uid, target_gid, e
            );
            return auth_misconfiguration();
        }

        if nix::unistd::ROOT == nix::unistd::getuid() {
            error!(
                "{} Crymap is still root! You must either \
                    (a) Run Crymap as a non-root user; \
                    (b) Set [security].system_user in crymap.toml; \
                    (c) Ensure that user directories are not owned by root.",
                self.log_prefix
            );
            return auth_misconfiguration();
        }

        Ok(())
    }
}

fn auth_misconfiguration() -> PartialResult<()> {
    Err(s::Response::Cond(s::CondResponse {
        cond: s::RespCondType::Bye,
        code: Some(s::RespTextCode::Alert(())),
        quip: Some(Cow::Borrowed(
            "Fatal internal error or misconfiguration; refer to \
             server logs for details.",
        )),
    }))
}