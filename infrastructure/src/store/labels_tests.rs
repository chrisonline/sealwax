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

/// A server account (it has a key), so its label changes wait for the op runner.
fn server_account(store: &Store) -> AccountId {
    store
        .ensure_account(
            sealwax_domain::account::AccountKey("labels-test".into()),
            NewAccount {
                name: "Ada".into(),
                address: "ada@example.test".into(),
            },
        )
        .recv_blocking()
        .unwrap()
        .value
}

fn label(store: &Store, messages: &[MessageId], label: FolderId, add: bool) -> Queued {
    store
        .queue_label(QueueLabel {
            messages: messages.to_vec(),
            label,
            add,
        })
        .recv_blocking()
        .unwrap()
}

fn due(store: &Store, account: AccountId) -> Vec<OpGroup> {
    store
        .due_ops(account, i64::MAX / 2)
        .recv_blocking()
        .unwrap()
        .groups
}

fn labels_of(store: &Store, id: MessageId) -> Vec<String> {
    store
        .message_labels(vec![id])
        .recv_blocking()
        .unwrap()
        .into_iter()
        .flat_map(|(_, labels)| labels)
        .collect()
}

#[test]
fn adding_a_label_copies_the_message_and_shows_at_once() {
    let (_dir, store) = open();
    let ada = server_account(&store);
    let ids = folders(
        &store,
        ada,
        &[
            ("INBOX", FolderRole::Inbox),
            ("Labels", FolderRole::Other),
            ("Labels/Work", FolderRole::Other),
        ],
    );
    let (inbox, work) = (ids[0], ids[2]);
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
    arrive(&store, inbox, message(inbox, 1, Some("a@example.test")));
    while let Ok(event) = events.try_recv() {
        model.apply(&store, &event);
    }
    let id = model.rows[0].id;

    let queued = label(&store, &[id], work, true);
    assert_eq!(queued.applied, 1);
    assert!(queued.batch.is_some());
    assert_eq!(queued.accounts, [ada]);
    while let Ok(event) = events.try_recv() {
        model.apply(&store, &event);
    }
    assert_eq!(
        labels(&model),
        [("Message 1".to_owned(), vec!["Work".to_owned()])]
    );
    assert_eq!(whole(&store, ViewSpec::folder(inbox)).rows, model.rows);
    assert_eq!(labels_of(&store, id), ["Work"]);
    let groups = due(&store, ada);
    assert_eq!(groups.len(), 1);
    assert!(
        matches!(groups[0].action, OpAction::Copy { to, .. } if to == work),
        "{:?}",
        groups[0].action
    );

    // Already on its way: nothing more is queued.
    let again = label(&store, &[id], work, true);
    assert_eq!((again.applied, again.batch), (1, None));
    assert_eq!(due(&store, ada).len(), 1);

    // Taken off before it started: the copy is dropped, and the label goes at once.
    let off = label(&store, &[id], work, false);
    assert_eq!(off.applied, 1);
    while let Ok(event) = events.try_recv() {
        model.apply(&store, &event);
    }
    assert_eq!(labels(&model), [("Message 1".to_owned(), vec![])]);
    assert!(due(&store, ada).is_empty());
}

#[test]
fn taking_a_label_off_expunges_the_copy_in_the_label_folder() {
    let (_dir, store) = open();
    let ada = server_account(&store);
    let ids = folders(
        &store,
        ada,
        &[
            ("INBOX", FolderRole::Inbox),
            ("Labels", FolderRole::Other),
            ("Labels/Work", FolderRole::Other),
            ("Labels/Reise", FolderRole::Other),
        ],
    );
    let (inbox, work, reise) = (ids[0], ids[2], ids[3]);
    arrive(&store, inbox, message(inbox, 1, Some("a@example.test")));
    arrive(&store, work, message(work, 4, Some("a@example.test")));
    arrive(&store, reise, message(reise, 9, Some("a@example.test")));
    let rows = whole(&store, ViewSpec::folder(inbox)).rows;
    let id = rows[0].id;
    assert_eq!(rows[0].labels, ["Reise", "Work"]);

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
    let off = label(&store, &[id], work, false);
    assert_eq!((off.applied, off.accounts.as_slice()), (1, &[ada][..]));
    while let Ok(event) = events.try_recv() {
        model.apply(&store, &event);
    }
    assert_eq!(model.rows[0].labels, ["Reise"]);
    let groups = due(&store, ada);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].folder, work);
    assert_eq!(groups[0].action, OpAction::Expunge);
    assert_eq!(groups[0].messages[0].uid, 4);

    // Off already: nothing more.
    let again = label(&store, &[id], work, false);
    assert_eq!((again.applied, again.batch), (1, None));
}

#[test]
fn a_label_change_leaves_alone_what_it_cant_label() {
    let (_dir, store) = open();
    let ada = server_account(&store);
    let ids = folders(
        &store,
        ada,
        &[
            ("INBOX", FolderRole::Inbox),
            ("Labels", FolderRole::Other),
            ("Labels/Work", FolderRole::Other),
        ],
    );
    let (inbox, work) = (ids[0], ids[2]);
    arrive(&store, inbox, message(inbox, 1, None));
    arrive(&store, inbox, message(inbox, 2, Some("b@example.test")));
    let rows = whole(&store, ViewSpec::folder(inbox)).rows;
    let (with_id, without) = (rows[0].id, rows[1].id);

    let queued = label(&store, &[without], work, true);
    assert_eq!(
        queued.not_applied,
        [(NotApplied::NoTarget, 1)],
        "no Message-ID"
    );
    let queued = label(&store, &[with_id], inbox, true);
    assert_eq!(
        queued.not_applied,
        [(NotApplied::NoTarget, 1)],
        "not a label folder"
    );
    assert!(due(&store, ada).is_empty());

    // A copy the runner has started can't be dropped.
    label(&store, &[with_id], work, true);
    let ops = due(&store, ada)[0].ops.clone();
    store.start_ops(ops, None).recv_blocking().unwrap();
    let queued = label(&store, &[with_id], work, false);
    assert_eq!(queued.not_applied, [(NotApplied::Busy, 1)]);

    // A local account has no server to copy on.
    let local = account(&store, "Bo");
    let theirs = folders(
        &store,
        local,
        &[
            ("INBOX", FolderRole::Inbox),
            ("Labels/Privat", FolderRole::Other),
        ],
    );
    arrive(
        &store,
        theirs[0],
        message(theirs[0], 1, Some("c@example.test")),
    );
    let row = whole(&store, ViewSpec::folder(theirs[0])).rows[0].id;
    let queued = label(&store, &[row], theirs[1], true);
    assert_eq!(queued.not_applied, [(NotApplied::NoTarget, 1)]);
}
