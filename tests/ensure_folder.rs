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

//! Integration test for Bichon coherence rule #3 — `ensure_folder_exists`.
//!
//! Deletes a `Berger/*` folder, triggers a `copy_to` action, and checks
//! the folder is recreated and the message deposited (CLAUDE.md §4.4).
//! Runs against a Greenmail IMAP server in a Docker container, so it
//! needs a running Docker daemon.

use std::time::Duration;

use async_imap::Session;
use berger::actions::imap_target::ImapActionTarget;
use berger::actions::{Action, apply_actions};
use futures_util::TryStreamExt;
use testcontainers::core::IntoContainerPort;
use testcontainers::runners::AsyncRunner;
use testcontainers::{GenericImage, ImageExt};
use tokio::net::TcpStream;

/// Greenmail's default IMAP test port.
const IMAP_PORT: u16 = 3143;

const TEST_MESSAGE: &str = "From: sender@example.test\r\n\
                            To: berger@localhost\r\n\
                            Subject: Berger integration test\r\n\
                            Message-ID: <ensure-folder@berger.test>\r\n\
                            \r\n\
                            Integration test body.\r\n";

/// One attempt to open and authenticate an IMAP session against Greenmail.
async fn try_connect(host: &str, port: u16) -> Option<Session<TcpStream>> {
    let tcp = TcpStream::connect((host, port)).await.ok()?;
    let mut client = async_imap::Client::new(tcp);
    client.read_response().await.ok()??;
    client.login("berger", "berger").await.ok()
}

/// Opens an IMAP session, retrying until Greenmail is ready.
async fn connect(host: &str, port: u16) -> Session<TcpStream> {
    for _ in 0..60 {
        if let Some(session) = try_connect(host, port).await {
            return session;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("Greenmail IMAP did not become ready in time");
}

#[tokio::test]
async fn copy_to_recreates_a_user_deleted_folder() {
    // A Greenmail IMAP server; auth disabled so any login is accepted.
    let container = GenericImage::new("greenmail/standalone", "2.1.0")
        .with_exposed_port(IMAP_PORT.tcp())
        .with_env_var(
            "GREENMAIL_OPTS",
            // greenmail.hostname=0.0.0.0 — bind every service to all
            // interfaces, otherwise the mapped port is unreachable.
            "-Dgreenmail.setup.test.all -Dgreenmail.hostname=0.0.0.0 -Dgreenmail.auth.disabled -Dgreenmail.verbose",
        )
        .start()
        .await
        .expect("Greenmail container should start");
    let host = container
        .get_host()
        .await
        .expect("container host")
        .to_string();
    let port = container
        .get_host_port_ipv4(IMAP_PORT.tcp())
        .await
        .expect("mapped IMAP port");

    // A session for test setup and verification, kept separate from the
    // one the action engine drives.
    let mut setup = connect(&host, port).await;

    // Discover the server's mailbox hierarchy separator (Greenmail: `.`).
    let names: Vec<_> = setup
        .list(None, None)
        .await
        .expect("LIST")
        .try_collect()
        .await
        .expect("collect mailbox names");
    let separator = names
        .first()
        .and_then(|name| name.delimiter())
        .unwrap_or("/")
        .to_string();
    let triage = format!("Berger{separator}triage");

    // Put a message in INBOX so there is a UID to act on.
    setup.select("INBOX").await.expect("select INBOX");
    setup
        .append("INBOX", None, None, TEST_MESSAGE)
        .await
        .expect("append the test message");
    setup.select("INBOX").await.expect("re-select INBOX");
    let uids = setup.uid_search("ALL").await.expect("UID SEARCH");
    let uid = *uids.iter().next().expect("one message in INBOX");

    // Simulate a user creating, then deleting, the Berger writeback folder.
    setup.create("Berger").await.expect("create Berger");
    setup.create(&triage).await.expect("create Berger/triage");
    setup.delete(&triage).await.expect("delete Berger/triage");

    // The action engine copies the message into Berger/triage — which no
    // longer exists, so ensure_folder_exists must recreate it (rule #3).
    let action_session = connect(&host, port).await;
    let mut target = ImapActionTarget::new(action_session)
        .await
        .expect("wrap the action session");
    apply_actions(&mut target, uid, &[Action::CopyTo("triage".to_string())])
        .await
        .expect("apply the copy_to action");

    // The folder was recreated and the message landed in it.
    let mailbox = setup
        .select(&triage)
        .await
        .expect("Berger/triage should have been recreated");
    assert_eq!(
        mailbox.exists, 1,
        "the message should have been copied into the recreated folder"
    );
}
