// Berger: open-source email triage daemon.
// Copyright (C) 2026 Michel-Marie Maudet
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

//! Inbox scan (PRD v1.1): a strictly read-only analysis of the inbox.
//!
//! `berger scan` observes the user's inbox over a recent window, measures
//! the recurring patterns in it — frequent senders, domains, newsletters,
//! notification services, ... — and proposes a starting `berger.yaml` for
//! the user to review and merge by hand.
//!
//! The scan is orthogonal to the triage pipeline: it never applies an IMAP
//! action, never calls the LLM, and never reads a message body. It reads
//! exclusively through [`source::ReadOnlyMessageSource`], a trait with no
//! mutating method, so the read-only guarantee holds at compile time.

pub mod address;
pub mod analyzers;
pub mod source;
