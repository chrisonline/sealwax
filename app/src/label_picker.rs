//! Label… (ADR-013 amendment of 2026-11-04): a keyboard-first picker over the labels of the
//! selected messages' account, its label folders (`Labels/…`, as Proton Mail Bridge lists them),
//! each with a check: ticked when every selected message has the label, half ticked when some
//! have it. Typing filters with Stet's matcher, Up and Down move, Enter or a click toggles the
//! label on all of them, and the picker stays open for the next one; Escape closes. It is an
//! `AdwDialog` like Move to…, so its entry keeps every letter.

use crate::actions::ActionSet;
use crate::application::AppState;
use crate::triage::Target;
use gtk4 as gtk;
use gtk4::prelude::*;
use gtk4::{gdk, glib};
use libadwaita as adw;
use libadwaita::prelude::*;
use sealwax_domain::actions::ActionId;
use sealwax_domain::folders;
use sealwax_domain::model::{AccountId, Folder, FolderId, MessageId};
use sealwax_domain::palette;
use sealwax_engine::api::Command;
use sealwax_infrastructure::store::{NotApplied, QueueLabel};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

/// Opens the picker on the selected messages, or on the selected group's mail.
pub fn install_window(state: &Rc<AppState>, actions: &ActionSet) {
    let weak = Rc::downgrade(state);
    actions.connect(ActionId::Label, move || {
        if let Some(state) = weak.upgrade() {
            open(&state);
        }
    });
}

/// Whether Label… applies to the selected messages, or why not.
pub fn applies(state: &AppState) -> Result<(), &'static str> {
    let list = state.list().ok_or("Select a message")?;
    let targets = crate::application::loaded_targets(&list);
    let folders = state.mailbox().folders();
    let accounts = state.mailbox().accounts();
    label_account(&targets, &folders, &accounts).map(|_| ())
}

/// The one server account `targets` belong to, if it has labels; else why Label… doesn't apply.
fn label_account(
    targets: &[Target],
    folders: &[Folder],
    accounts: &[sealwax_domain::model::Account],
) -> Result<AccountId, &'static str> {
    let mut account = None;
    for target in targets {
        let Some(folder) = folders.iter().find(|folder| folder.id == target.folder) else {
            return Err("Some selected messages haven't loaded");
        };
        match account {
            None => account = Some(folder.account),
            Some(other) if other != folder.account => {
                return Err("Labels apply to one account's mail at a time");
            }
            Some(_) => {}
        }
    }
    let account = account.ok_or("Select a message")?;
    if !accounts
        .iter()
        .any(|known| known.id == account && known.key.is_some())
    {
        return Err("Labels need an account on a mail server");
    }
    if !folders.iter().any(|folder| {
        folder.account == account
            && folder.selectable
            && folders::label(&folder.name, folder.delimiter).is_some()
    }) {
        return Err("The account has no labels");
    }
    Ok(account)
}

fn open(state: &Rc<AppState>) {
    let Some(list) = state.list() else {
        return;
    };
    let weak = Rc::downgrade(state);
    glib::spawn_future_local(async move {
        let targets = if list.selected_group().is_some() {
            list.fresh_group_targets().await
        } else {
            match list.selected_targets() {
                Ok(targets) => Some(targets),
                Err(_) => list.wait_targets().await,
            }
        };
        let Some(state) = weak.upgrade() else {
            return;
        };
        let Some(targets) = targets.filter(|targets| !targets.is_empty()) else {
            toast(
                &state,
                "The selected messages didn't load; no label changed",
            );
            return;
        };
        let folders = state.mailbox().folders();
        let account = match label_account(&targets, &folders, &state.mailbox().accounts()) {
            Ok(account) => account,
            Err(why) => {
                toast(&state, why);
                return;
            }
        };
        let ids: Vec<MessageId> = targets.iter().map(|target| target.id).collect();
        let reply = state
            .mailbox()
            .store()
            .map(|store| store.message_labels(ids.clone()));
        let labels = match reply {
            Some(reply) => reply.recv().await,
            None => return,
        };
        let Ok(labels) = labels else {
            toast(&state, "The labels didn't load");
            return;
        };
        let labels: BTreeMap<MessageId, Vec<String>> = labels.into_iter().collect();
        let account_folders: Vec<Folder> = folders
            .into_iter()
            .filter(|folder| folder.account == account)
            .collect();
        let choices = choices(&account_folders, &labels);
        let Some(window) = state.window() else {
            return;
        };
        let weak = Rc::downgrade(&state);
        let count = u32::try_from(ids.len()).unwrap_or(u32::MAX);
        present(window.window(), choices, count, move |label, add| {
            if let Some(state) = weak.upgrade() {
                queue(&state, ids.clone(), label, add);
            }
        });
    });
}

/// Adds `label` to `messages` or takes it off, and wakes the account's op runner.
fn queue(state: &Rc<AppState>, messages: Vec<MessageId>, label: FolderId, add: bool) {
    let reply = state.mailbox().store().map(|store| {
        store.queue_label(QueueLabel {
            messages,
            label,
            add,
        })
    });
    let Some(reply) = reply else {
        return;
    };
    let weak = Rc::downgrade(state);
    glib::spawn_future_local(async move {
        let answer = reply.recv().await;
        let Some(state) = weak.upgrade() else {
            return;
        };
        let queued = match answer {
            Ok(queued) => queued,
            Err(error) => {
                tracing::warn!("the store refused a label change: {error}");
                toast(&state, "The label didn't change");
                return;
            }
        };
        for account in &queued.accounts {
            if state
                .engine()
                .send(Command::OpsQueued { account: *account })
                .is_err()
            {
                tracing::debug!(account = account.0, "no engine to run the label change");
            }
        }
        if let Some(text) = left_text(&queued.not_applied) {
            toast(&state, &text);
        }
    });
}

/// What a label change left alone, in a toast's words.
fn left_text(not_applied: &[(NotApplied, u32)]) -> Option<String> {
    let parts: Vec<String> = not_applied
        .iter()
        .filter(|(_, count)| *count > 0)
        .map(|&(why, count)| {
            let n = if count == 1 {
                "1 message".to_owned()
            } else {
                format!("{count} messages")
            };
            match why {
                NotApplied::Busy => format!("{n} still being labelled; try again in a moment"),
                NotApplied::Gone => format!("{n} already gone"),
                _ => format!("{n} can't be labelled"),
            }
        })
        .collect();
    (!parts.is_empty()).then(|| parts.join(". "))
}

fn toast(state: &AppState, text: &str) {
    let toast = adw::Toast::builder()
        .title(text)
        .use_markup(false)
        .timeout(5)
        .build();
    state.show_toast(&toast, false);
}

/// How many of the selected messages have a label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Have {
    All,
    Some,
    None,
}

impl Have {
    /// Whether toggling adds the label (to every message) rather than removing it (from every
    /// message).
    pub fn toggle_adds(self) -> bool {
        self != Have::All
    }

    fn spoken(self) -> &'static str {
        match self {
            Have::All => "labelled",
            Have::Some => "on some of them",
            Have::None => "not labelled",
        }
    }
}

/// One label the picker offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    /// The label folder.
    pub folder: FolderId,
    pub name: String,
    pub have: Have,
}

impl Choice {
    fn spoken(&self) -> String {
        format!("{}, {}", self.name, self.have.spoken())
    }
}

/// The labels of `account_folders` (one account's) for messages whose labels are `labels`, by
/// name, in the order they sort in on the rows.
pub fn choices(
    account_folders: &[Folder],
    labels: &BTreeMap<MessageId, Vec<String>>,
) -> Vec<Choice> {
    let total = labels.len();
    let mut choices: Vec<Choice> = account_folders
        .iter()
        .filter(|folder| folder.selectable)
        .filter_map(|folder| {
            let name = folders::label(&folder.name, folder.delimiter)?;
            let with = labels
                .values()
                .filter(|have| have.iter().any(|label| label == name))
                .count();
            let have = match with {
                0 => Have::None,
                with if with == total => Have::All,
                _ => Have::Some,
            };
            Some(Choice {
                folder: folder.id,
                name: name.to_owned(),
                have,
            })
        })
        .collect();
    choices.sort_by_cached_key(|choice| (choice.name.to_lowercase(), choice.name.clone()));
    choices
}

/// What toggling a label asks for: the label folder, and whether it is added (else removed).
type Toggle = Rc<dyn Fn(FolderId, bool)>;

struct Inner {
    dialog: glib::WeakRef<adw::Dialog>,
    entry: glib::WeakRef<gtk::SearchEntry>,
    list: glib::WeakRef<gtk::ListBox>,
    scrolled: glib::WeakRef<gtk::ScrolledWindow>,
    choices: RefCell<Vec<Choice>>,
    /// Indices into `choices`, best match first.
    shown: RefCell<Vec<usize>>,
    /// The row Enter toggles, kept while a toggle redraws the rows.
    selected: Cell<usize>,
    toggle: Toggle,
}

thread_local! {
    /// The picker open now, for the self-test.
    static OPEN: RefCell<Option<LabelPicker>> = const { RefCell::new(None) };
}

/// An open picker; [`present`] makes one.
#[derive(Clone)]
pub struct LabelPicker {
    dialog: adw::Dialog,
    inner: Rc<Inner>,
}

/// Opens the picker over `window` for `count` messages, offering `choices` in their order;
/// `toggle` gets each label toggled and whether it was added.
pub fn present(
    window: &impl IsA<gtk::Widget>,
    choices: Vec<Choice>,
    count: u32,
    toggle: impl Fn(FolderId, bool) + 'static,
) -> LabelPicker {
    let picker = LabelPicker::new(choices, count, Rc::new(toggle));
    picker.dialog.present(Some(window));
    if let Some(entry) = picker.inner.entry.upgrade() {
        entry.grab_focus();
    }
    OPEN.with(|open| open.replace(Some(picker.clone())));
    picker
}

/// The picker open now, if any.
pub fn current() -> Option<LabelPicker> {
    OPEN.with(|open| open.borrow().clone())
        .filter(|picker| picker.dialog.is_mapped())
}

impl LabelPicker {
    fn new(choices: Vec<Choice>, count: u32, toggle: Toggle) -> Self {
        let title = match count {
            1 => "Label 1 message".to_owned(),
            count => format!("Label {count} messages"),
        };
        let entry = gtk::SearchEntry::builder()
            .placeholder_text(title.as_str())
            .search_delay(0)
            .hexpand(true)
            .css_classes(["sw-palette-search", "sw-label-picker-search"])
            .build();
        entry.set_widget_name("label-picker-search");
        entry.update_property(&[gtk::accessible::Property::Label(&title)]);
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Browse)
            .css_classes(["sw-palette-list", "sw-label-picker-list"])
            .build();
        list.set_widget_name("label-picker-list");
        list.update_property(&[gtk::accessible::Property::Label("Labels")]);
        let empty = gtk::Label::builder()
            .label("No matching label")
            .css_classes(["sw-palette-empty", "sw-secondary"])
            .build();
        list.set_placeholder(Some(&empty));
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&list)
            .focusable(false)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        let header = adw::HeaderBar::builder().title_widget(&entry).build();
        let content = adw::ToolbarView::new();
        content.add_top_bar(&header);
        content.set_content(Some(&scrolled));
        let dialog = adw::Dialog::builder()
            .title(title.as_str())
            .content_width(360)
            .content_height(380)
            .child(&content)
            .focus_widget(&entry)
            .css_classes(["sw-palette", "sw-label-picker"])
            .build();
        dialog.set_widget_name("label-picker");
        let inner = Rc::new(Inner {
            dialog: dialog.downgrade(),
            entry: entry.downgrade(),
            list: list.downgrade(),
            scrolled: scrolled.downgrade(),
            choices: RefCell::new(choices),
            shown: RefCell::new(Vec::new()),
            selected: Cell::new(0),
            toggle,
        });
        connect(&inner, &entry, &list);
        inner.refresh();
        Self { dialog, inner }
    }

    pub fn dialog(&self) -> &adw::Dialog {
        &self.dialog
    }

    pub fn search_entry(&self) -> Option<gtk::SearchEntry> {
        self.inner.entry.upgrade()
    }

    /// The labels shown, best match first, with how many of the messages have each.
    pub fn shown(&self) -> Vec<(String, Have)> {
        let choices = self.inner.choices.borrow();
        self.inner
            .shown
            .borrow()
            .iter()
            .map(|index| (choices[*index].name.clone(), choices[*index].have))
            .collect()
    }

    /// Toggles the label Enter would toggle, as Enter does.
    pub fn toggle_selected(&self) {
        self.inner.toggle_selected();
    }
}

fn connect(inner: &Rc<Inner>, entry: &gtk::SearchEntry, list: &gtk::ListBox) {
    entry.connect_search_changed(glib::clone!(
        #[strong]
        inner,
        move |_| {
            inner.selected.set(0);
            inner.refresh();
        }
    ));
    entry.connect_activate(glib::clone!(
        #[strong]
        inner,
        move |_| inner.toggle_selected()
    ));
    entry.connect_stop_search(glib::clone!(
        #[strong]
        inner,
        move |_| {
            if let Some(dialog) = inner.dialog.upgrade() {
                dialog.close();
            }
        }
    ));
    list.connect_row_activated(glib::clone!(
        #[strong]
        inner,
        move |_, row| {
            if let Ok(index) = usize::try_from(row.index()) {
                inner.selected.set(index);
                inner.toggle_selected();
            }
            if let Some(entry) = inner.entry.upgrade() {
                entry.grab_focus();
            }
        }
    ));
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(glib::clone!(
        #[strong]
        inner,
        move |_, key, _, _| {
            let step = match key {
                gdk::Key::Down => 1,
                gdk::Key::Up => -1,
                gdk::Key::Page_Down => 8,
                gdk::Key::Page_Up => -8,
                _ => return glib::Propagation::Proceed,
            };
            inner.move_selection(step);
            glib::Propagation::Stop
        }
    ));
    entry.add_controller(keys);
}

impl Inner {
    /// Ranks the labels for the entry's text and draws them, the selected row kept.
    fn refresh(&self) {
        let (Some(entry), Some(list)) = (self.entry.upgrade(), self.list.upgrade()) else {
            return;
        };
        let query = entry.text();
        let shown: Vec<usize> = {
            let choices = self.choices.borrow();
            palette::rank(&query, choices.iter().map(|choice| choice.name.as_str()))
        };
        list.remove_all();
        {
            let choices = self.choices.borrow();
            for index in &shown {
                list.append(&row(&choices[*index]));
            }
        }
        let selected = self.selected.get().min(shown.len().saturating_sub(1));
        self.selected.set(selected);
        self.shown.replace(shown);
        if let Some(row) = i32::try_from(selected)
            .ok()
            .and_then(|index| list.row_at_index(index))
        {
            list.select_row(Some(&row));
        }
    }

    fn toggle_selected(&self) {
        let Some(index) = self.shown.borrow().get(self.selected.get()).copied() else {
            return;
        };
        let (folder, add, spoken) = {
            let mut choices = self.choices.borrow_mut();
            let choice = &mut choices[index];
            let add = choice.have.toggle_adds();
            choice.have = if add { Have::All } else { Have::None };
            (choice.folder, add, choice.spoken())
        };
        self.refresh();
        if let Some(entry) = self.entry.upgrade() {
            crate::accessibility::announce(&entry, &spoken);
        }
        (self.toggle)(folder, add);
    }

    fn move_selection(&self, step: i32) {
        let Some(list) = self.list.upgrade() else {
            return;
        };
        let count = i32::try_from(self.shown.borrow().len()).unwrap_or(i32::MAX);
        if count == 0 {
            return;
        }
        let current = list.selected_row().map_or(-1, |row| row.index());
        let next = (current + step).clamp(0, count - 1);
        let Some(row) = list.row_at_index(next) else {
            return;
        };
        list.select_row(Some(&row));
        self.selected.set(usize::try_from(next).unwrap_or(0));
        // The focus stays in the entry, so a screen reader hears the row this way.
        let spoken = self
            .shown
            .borrow()
            .get(self.selected.get())
            .map(|index| self.choices.borrow()[*index].spoken());
        if let (Some(entry), Some(spoken)) = (self.entry.upgrade(), spoken) {
            crate::accessibility::announce(&entry, &spoken);
        }
        if let (Some(scrolled), Some(bounds)) = (self.scrolled.upgrade(), row.compute_bounds(&list))
        {
            let adjustment = scrolled.vadjustment();
            let (top, bottom) = (
                f64::from(bounds.y()),
                f64::from(bounds.y() + bounds.height()),
            );
            if top < adjustment.value() {
                adjustment.set_value(top);
            } else if bottom > adjustment.value() + adjustment.page_size() {
                adjustment.set_value(bottom - adjustment.page_size());
            }
        }
    }
}

fn row(choice: &Choice) -> gtk::ListBoxRow {
    let check = gtk::CheckButton::builder()
        .active(choice.have == Have::All)
        .inconsistent(choice.have == Have::Some)
        .can_focus(false)
        .can_target(false)
        .valign(gtk::Align::Center)
        .build();
    let chips = gtk::Box::builder()
        .hexpand(true)
        .halign(gtk::Align::Start)
        .build();
    crate::labels::fill(&chips, std::slice::from_ref(&choice.name), 1);
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .margin_start(10)
        .margin_end(10)
        .margin_top(6)
        .margin_bottom(6)
        .build();
    content.append(&check);
    content.append(&chips);
    let row = gtk::ListBoxRow::builder()
        .child(&content)
        .css_classes(["sw-palette-row", "sw-label-picker-row"])
        .build();
    // The focus stays in the search entry, where Up and Down move the chosen row.
    row.set_focusable(false);
    row.update_property(&[gtk::accessible::Property::Label(&choice.spoken())]);
    row
}

#[cfg(test)]
mod tests {
    use super::*;
    use sealwax_domain::model::{AccountId, FolderRole};

    fn folder(id: i64, name: &str, selectable: bool) -> Folder {
        Folder {
            id: FolderId(id),
            account: AccountId(1),
            name: name.to_owned(),
            delimiter: Some('/'),
            role: FolderRole::Other,
            selectable,
            inferiors: true,
            total: 0,
            unread: 0,
        }
    }

    #[test]
    fn what_a_label_change_left_alone() {
        assert_eq!(left_text(&[]), None);
        assert_eq!(
            left_text(&[(NotApplied::NoTarget, 1), (NotApplied::Busy, 2)]).as_deref(),
            Some(
                "1 message can't be labelled. 2 messages still being labelled; try again in a moment"
            )
        );
    }

    #[test]
    fn every_label_folder_with_how_many_messages_have_it() {
        let account = vec![
            folder(1, "INBOX", true),
            folder(2, "Labels", false),
            folder(3, "Labels/work", true),
            folder(4, "Labels/Reise", true),
            folder(5, "Labels/Privat", true),
            folder(6, "Folders/Work", true),
        ];
        let labels = BTreeMap::from([
            (MessageId(10), vec!["Reise".to_owned(), "work".to_owned()]),
            (MessageId(11), vec!["Reise".to_owned()]),
        ]);
        let got: Vec<(String, Have)> = choices(&account, &labels)
            .into_iter()
            .map(|choice| (choice.name, choice.have))
            .collect();
        assert_eq!(
            got,
            [
                ("Privat".to_owned(), Have::None),
                ("Reise".to_owned(), Have::All),
                ("work".to_owned(), Have::Some),
            ]
        );
        assert!(Have::None.toggle_adds());
        assert!(Have::Some.toggle_adds(), "half ticked adds it to the rest");
        assert!(!Have::All.toggle_adds());
        let reise = choices(&account, &labels).remove(1);
        assert_eq!(reise.folder, FolderId(4));
        assert_eq!(reise.spoken(), "Reise, labelled");
    }
}
