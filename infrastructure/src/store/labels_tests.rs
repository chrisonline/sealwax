//! Labels (ADR-013 amendment of 2026-10-19): a message's labels are the label folders of its
//! account holding a copy of its Message-ID, as Proton Mail Bridge lists them, and the rows of
//! its other copies follow when a copy comes into a label folder or leaves one.

use super::tests::{Model, scratch, whole};
use super::*;
use sealwax_domain::model::{FolderRole, Headers};

fn open() -> (tempfile::TempDir, Store) {
    let dir = scratch();
    let store = Store::open(&dir.path().join("store.sqlite"), StoreOptions::default()).unwrap();
    (dir, store)
}

fn account(store: &Store, name: &str) -> AccountId {
    store
        .add_account(NewAccount {
            name: name.into(),
            address: format!("{}@example.test", name.to_lowercase()),
        })
        .recv_blocking()
        .unwrap()
        .value
}

/// The account's folders, each ready for sync passes, in the order named.
fn folders(store: &Store, account: AccountId, names: &[(&str, FolderRole)]) -> Vec<FolderId> {
    let mut specs: Vec<FolderSpec> = names
        .iter()
        .map(|(name, role)| FolderSpec::local(account, name, *role))
        .collect();
    if let Some(container) = specs.iter_mut().find(|spec| spec.name == "Labels") {
        container.selectable = false;
    }
    let ids = store.upsert_folders(specs).recv_blocking().unwrap().value;
    for (id, (name, _)) in ids.iter().zip(names) {
        if *name == "Labels" {
            continue;
        }
        store
            .first_sync_step(
                *id,
                FirstSyncStep::Start {
                    uidvalidity: 7,
                    top: 1,
                    modseq: 1,
                },
            )
            .recv_blocking()
            .unwrap();
        store
            .first_sync_step(*id, FirstSyncStep::Finish { below: 0 })
            .recv_blocking()
            .unwrap();
    }
    ids
}

fn message(folder: FolderId, uid: u32, message_id: Option<&str>) -> NewMessage {
    NewMessage {
        folder,
        uid: Some(uid),
        flags: Flags::SEEN,
        date: 1_790_000_000 + i64::from(uid),
        headers: Headers {
            subject: format!("Message {uid}"),
            message_id: message_id.map(str::to_owned),
            ..Headers::default()
        },
        size: 100,
        ..NewMessage::default()
    }
}

fn arrive(store: &Store, folder: FolderId, new: NewMessage) {
    store
        .sync_changes(
            folder,
            FolderChanges {
                new: vec![new],
                ..FolderChanges::default()
            },
        )
        .recv_blocking()
        .unwrap();
}

fn leave(store: &Store, folder: FolderId, uid: u32) {
    store
        .sync_changes(
            folder,
            FolderChanges {
                removed: vec![uid],
                ..FolderChanges::default()
            },
        )
        .recv_blocking()
        .unwrap();
}

fn labels(model: &Model) -> Vec<(String, Vec<String>)> {
    model
        .rows
        .iter()
        .map(|row| (row.subject.clone(), row.labels.clone()))
        .collect()
}

#[test]
fn copies_in_label_folders_label_the_inbox_rows_and_the_rows_follow() {
    let (_dir, store) = open();
    let ada = account(&store, "Ada");
    let ids = folders(
        &store,
        ada,
        &[
            ("INBOX", FolderRole::Inbox),
            ("Labels", FolderRole::Other),
            ("Labels/Work", FolderRole::Other),
            ("Labels/Reise", FolderRole::Other),
            ("Folders/Work", FolderRole::Other),
        ],
    );
    let (inbox, work, reise, not_a_label) = (ids[0], ids[2], ids[3], ids[4]);
    let other = account(&store, "Bo");
    let theirs = folders(&store, other, &[("Labels/Privat", FolderRole::Other)])[0];

    let events = store.subscribe();
    let opened = store
        .open_view(OpenView {
            spec: ViewSpec::folder(inbox),
            first_page: u32::MAX,
            section_starts: Vec::new(),
        })
        .recv_blocking()
        .unwrap();
    let mut model = Model {
        view: opened.view,
        rows: opened.rows,
        version: opened.version,
    };
    let check = |model: &mut Model, want: &[(&str, &[&str])]| {
        while let Ok(event) = events.try_recv() {
            model.apply(&store, &event);
        }
        let want: Vec<(String, Vec<String>)> = want
            .iter()
            .map(|(subject, labels)| {
                (
                    (*subject).to_owned(),
                    labels.iter().map(|label| (*label).to_owned()).collect(),
                )
            })
            .collect();
        assert_eq!(labels(model), want, "from change events");
        let read = whole(&store, ViewSpec::folder(inbox));
        assert_eq!(read.rows, model.rows, "as read again");
    };

    arrive(&store, inbox, message(inbox, 1, Some("a@example.test")));
    arrive(&store, inbox, message(inbox, 2, None));
    check(&mut model, &[("Message 2", &[]), ("Message 1", &[])]);

    arrive(&store, work, message(work, 1, Some("a@example.test")));
    check(&mut model, &[("Message 2", &[]), ("Message 1", &["Work"])]);

    // Another folder, another account's label, a message without a Message-ID: no label.
    arrive(
        &store,
        not_a_label,
        message(not_a_label, 1, Some("a@example.test")),
    );
    arrive(&store, theirs, message(theirs, 1, Some("a@example.test")));
    arrive(&store, reise, message(reise, 7, None));
    check(&mut model, &[("Message 2", &[]), ("Message 1", &["Work"])]);

    arrive(&store, reise, message(reise, 8, Some("a@example.test")));
    check(
        &mut model,
        &[("Message 2", &[]), ("Message 1", &["Reise", "Work"])],
    );

    leave(&store, work, 1);
    check(&mut model, &[("Message 2", &[]), ("Message 1", &["Reise"])]);

    // The label folder's own row carries its label too.
    let page = whole(&store, ViewSpec::folder(reise));
    let row = page
        .rows
        .iter()
        .find(|row| row.subject == "Message 8")
        .unwrap();
    assert_eq!(row.labels, ["Reise"]);
}
