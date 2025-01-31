---
-- Copyright (c) 2023, 2024, Jason Lingle
--
-- This file is part of Crymap.
--
-- Crymap is free software: you can  redistribute it and/or modify it under the
-- terms of  the GNU General Public  License as published by  the Free Software
-- Foundation, either version  3 of the License, or (at  your option) any later
-- version.
--
-- Crymap is distributed  in the hope that  it will be useful,  but WITHOUT ANY
-- WARRANTY; without  even the implied  warranty of MERCHANTABILITY  or FITNESS
-- FOR  A PARTICULAR  PURPOSE.  See the  GNU General  Public  License for  more
-- details.
--
-- You should have received a copy of the GNU General Public License along with
-- Crymap. If not, see <http://www.gnu.org/licenses/>.

-- Defines the mailboxes owned by the user.
CREATE TABLE `mailbox` (
  -- The surrogate ID for this mailbox. This is also used for the MAILBOXID
  -- reported to the client and is also the `UIDVALIDITY` value.
  -- (`AUTOINCREMENT` starts at 1, so we don't end up with a `UIDVALIDITY` of
  -- 0.)
  `id` INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
  -- The parent of this mailbox, or NULL if this is under the root.
  `parent_id` INTEGER NOT NULL,
  -- The IMAP name for this mailbox. E.g. "INBOX"; "TPS Reports".
  `name` TEXT NOT NULL,
  -- Whether this mailbox is selectable. An unselectable mailbox is brought
  -- about by `DELETE`ing a mailbox which has children.
  `selectable` INTEGER NOT NULL DEFAULT TRUE,
  -- If this is a special-use mailbox, the special use attribute (e.g.
  -- "\Sent").
  `special_use` TEXT,
  -- The next UID to provision for a message.
  `next_uid` INTEGER NOT NULL DEFAULT 1,
  -- The UID of the first message to mark as "recent".
  `recent_uid` INTEGER NOT NULL DEFAULT 1,
  -- The latest modification sequence number that has been used in this
  -- mailbox. Modseq 1 is the initial state of the mailbox.
  `max_modseq` INTEGER NOT NULL DEFAULT 1,
  UNIQUE (`parent_id`, `name`),
  FOREIGN KEY (`parent_id`) REFERENCES `mailbox` (`id`) ON DELETE RESTRICT
) STRICT;

-- Pre-seed the special root pseudo-mailbox.
INSERT INTO `mailbox` (`id`, `parent_id`, `name`, `selectable`)
VALUES (0, 0, '/', false);

-- Defines the known flags for an account.
--
-- The set of extant flags is global across the whole account. This ensures
-- that the flag table of a message doesn't need to be rewritten when it is
-- moved to another mailbox and eliminates separate bookkeeping about which
-- flags exist for each specific mailbox.
CREATE TABLE `flag` (
  -- The integer ID for this flag.
  `id` INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
  -- The flag itself. E.g. "\Deleted", "\Sent", "Keyword".
  --
  -- Flags are ASCII-only so the built-in NOCASE collation is sufficient.
  `flag` TEXT COLLATE NOCASE NOT NULL,
  UNIQUE (`flag`)
) STRICT;

-- Pre-seed the database with the non-keyword flags. This ensures they have low
-- IDs and also addresses the fact that AUTOINCREMENT starts at 1; this way, we
-- can ensure bit 0 is used too without needing to do offsetting in code.
INSERT INTO `flag` (`id`, `flag`) VALUES
  -- We specifically make \Seen be bit 0 since the most common flag combination
  -- is simply \Seen (i.e. just \Seen is 1) and SQLite has a compact
  -- representation for integers 0 and 1.
  (0, '\Seen'),
  (1, '\Answered'),
  (2, '\Deleted'),
  (3, '\Draft'),
  (4, '\Flagged');

-- Tracks all messages which exist in the user's account.
--
-- Messages are not inherently associated with any mailbox, but are brought in
-- via `mailbox_message`. `COPY` and so forth merely add more references to the
-- same message.
CREATE TABLE `message` (
  -- The surrogate ID for this message. This is also used for the EMAILID
  -- return to the client.
  `id` INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
  -- The file path of this message relative to the root of the message store.
  --
  -- This is usually a hex SHA-3 hash with a `/` after the first octet, but we
  -- support arbitrary paths to simplify file recovery (i.e. an admin can just
  -- drop a file in with an arbitrary path).
  `path` TEXT NOT NULL,
  -- The 16-byte session key used to encrypt the message, if known. The session
  -- key is XOR'ed with a 16-bit KMAC derived from the master key unique to
  -- `id` so that breaking the weaker XEX encryption of the database does not
  -- compromise the session keys.
  `session_key` BLOB,
  -- The value of `RFC822.SIZE`, if known.
  `rfc822_size` INTEGER,
  -- The last time (as a UNIX timestamp, seconds) at which the message was
  -- expunged from any mailbox or had other activity that could result in it
  -- being orphaned. This is automatically maintained by the default value and
  -- triggers.
  --
  -- This is used to identify when it is safe to delete orphaned messages.
  `last_activity` INTEGER NOT NULL DEFAULT (unixepoch()),
  -- The number of references from `mailbox_message` to this message. This is
  -- maintained automatically by triggers.
  `refcount` INTEGER NOT NULL DEFAULT 0,
  -- `summary_bucket` and `summary_increment` are used to efficiently determine
  -- whether there could be any untracked files in the message store. They are
  -- based on hashes of `path`. `summary_bucket` ranges from 0..=255 and
  -- `summary_increment` from 1..=65535.
  `summary_bucket` INTEGER NOT NULL,
  `summary_increment` INTEGER NOT NULL,
  UNIQUE (`path`)
) STRICT;

CREATE INDEX `message_orphan_status` ON `message` (`refcount`, `last_activity`);
CREATE INDEX `message_summary` ON `message` (`summary_bucket`, `summary_increment`);

-- An instance of a message within a mailbox.
CREATE TABLE `mailbox_message` (
  -- The mailbox that contains the message.
  `mailbox_id` INTEGER NOT NULL,
  -- The UID of this instance within the mailbox.
  `uid` INTEGER NOT NULL,
  -- The message itself.
  `message_id` INTEGER NOT NULL,
  -- The first 64 flags as a bitset.
  `near_flags` INTEGER NOT NULL,
  -- The datetime (UNIX, seconds) at which this instance of the message was
  -- appended.
  `savedate` INTEGER NOT NULL,
  -- The modseq at which this instance of the message was appended.
  `append_modseq` INTEGER NOT NULL,
  -- The modseq at which the flags were changed.
  `flags_modseq` INTEGER NOT NULL,
  PRIMARY KEY (`mailbox_id`, `uid`),
  FOREIGN KEY (`mailbox_id`) REFERENCES `mailbox` (`id`) ON DELETE RESTRICT,
  FOREIGN KEY (`message_id`) REFERENCES `message` (`id`) ON DELETE RESTRICT
) WITHOUT ROWID, STRICT;

CREATE INDEX `mailbox_message_message_id` ON `mailbox_message` (`message_id`);

CREATE TRIGGER `mailbox_message_refcount_incr`
AFTER INSERT ON `mailbox_message`
FOR EACH ROW BEGIN
  UPDATE `message` SET `refcount` = `refcount` + 1
  WHERE `message`.`id` = NEW.`message_id`;
END;

CREATE TRIGGER `mailbox_message_refcount_decr`
AFTER DELETE ON `mailbox_message`
FOR EACH ROW BEGIN
  UPDATE `message`
  SET `refcount` = `refcount` - 1, `last_activity` = unixepoch()
  WHERE `message`.`id` = OLD.`message_id`;
END;

CREATE TRIGGER `mailbox_message_no_message_id_update`
BEFORE UPDATE ON `mailbox_message`
FOR EACH ROW WHEN OLD.`message_id` != NEW.`message_id` BEGIN
  SELECT RAISE (FAIL, 'updating message_id is not allowed');
END;

-- Associates flags with indices >63 with messages.
CREATE TABLE `mailbox_message_far_flag` (
  `mailbox_id` INTEGER NOT NULL,
  `uid` INTEGER NOT NULL,
  `flag_id` INTEGER NOT NULL,
  PRIMARY KEY (`mailbox_id`, `uid`, `flag_id`),
  FOREIGN KEY (`mailbox_id`, `uid`)
    REFERENCES `mailbox_message` (`mailbox_id`, `uid`) ON DELETE RESTRICT,
  FOREIGN KEY (`flag_id`) REFERENCES `flag` (`id`) ON DELETE RESTRICT
) WITHOUT ROWID, STRICT;

CREATE TABLE `mailbox_message_expungement` (
  `mailbox_id` INTEGER NOT NULL,
  `uid` INTEGER NOT NULL,
  `expunged_modseq` INTEGER NOT NULL,
  -- This primary key + WITHOUT ROWID means that it is extremely efficient to
  -- scan in everything that changed after a certain point within one mailbox,
  -- as such a query will be a binary search then a linear table scan to gather
  -- all the UIDs.
  PRIMARY KEY (`mailbox_id`, `expunged_modseq`, `uid`),
  FOREIGN KEY (`mailbox_id`) REFERENCES `mailbox` (`id`) ON DELETE RESTRICT
) WITHOUT ROWID, STRICT;

-- Tracks the set of subscribed mailbox paths.
CREATE TABLE `subscription` (
  `path` TEXT NOT NULL PRIMARY KEY
) STRICT;

-- Used to coordinate periodic maintenance operations.
CREATE TABLE `maintenance` (
  -- The type of maintenance in question.
  `name` TEXT NOT NULL PRIMARY KEY,
  -- The datetime (UNIX, seconds) at which a process last started this kind of
  -- maintenance.
  `last_started` INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

-- Tracks outbound messages that have not yet been completed.
CREATE TABLE `message_spool` (
  -- The message to be delivered.
  `message_id` INTEGER NOT NULL PRIMARY KEY,
  -- The suggested SMTP transfer.
  `transfer` TEXT NOT NULL,
  -- The MAIL FROM email address.
  `mail_from` TEXT NOT NULL,
  -- The UNIX timestamp at which this entry will be forgotten.
  `expires` INTEGER NOT NULL,
  FOREIGN KEY (`message_id`) REFERENCES `message` (`id`)
    ON DELETE RESTRICT
) STRICT;

CREATE INDEX `message_spool_message_id`
ON `message_spool` (`message_id`);

CREATE TRIGGER `message_spool_refcount_incr`
AFTER INSERT ON `message_spool`
FOR EACH ROW BEGIN
  UPDATE `message` SET `refcount` = `refcount` + 1
  WHERE `message`.`id` = NEW.`message_id`;
END;

CREATE TRIGGER `message_spool_refcount_decr`
BEFORE DELETE ON `message_spool`
FOR EACH ROW BEGIN
  -- No need to update last_activity because spooled messages don't need to
  -- linger.
  UPDATE `message` set `refcount` = `refcount` - 1
  WHERE `message`.`id` = OLD.`message_id`;
END;

-- Tracks the outstanding destinations for each `message_spool` row.
CREATE TABLE `message_spool_destination` (
  `message_id` INTEGER NOT NULL,
  -- The destination email address.
  `destination` TEXT NOT NULL,
  PRIMARY KEY (`message_id`, `destination`),
  FOREIGN KEY (`message_id`)
    REFERENCES `message_spool` (`message_id`)
    ON DELETE CASCADE
) STRICT;

-- Tracks the TLS support that has been seen on destination domains.
CREATE TABLE `foreign_smtp_tls_status` (
  -- The domain (Punycode) in question.
  `domain` TEXT NOT NULL PRIMARY KEY,
  -- Whether the STARTTLS extension is present and usable. Once a delivery has
  -- been made to a destination using STARTTLS, cleartext deliveries will fail.
  `starttls` INTEGER NOT NULL,
  -- Whether the site actually uses valid certificates. On the first use of
  -- TLS, literally any certificate will do, but once a fully valid certificate
  -- is seen, certificates are required to be valid.
  `valid_certificate` INTEGER NOT NULL,
  -- The maximum TLS version that has been negotiated.
  `tls_version` TEXT
) STRICT;
