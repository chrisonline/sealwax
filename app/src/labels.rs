//! A message's labels as chips (ADR-013 amendment of 2026-10-19), in the message list and the
//! reader: one chip each, in the colour `config.toml`'s `[labels]` gives it, else grey. The
//! colours are the account colours (`sw-label-<name>`, style.css), so they read on light and
//! dark themes alike.

use gtk4 as gtk;
use gtk4::prelude::*;
use sealwax_domain::account::AccountColor;
use std::cell::RefCell;
use std::collections::BTreeMap;

/// Characters a chip shows at most before it is ellipsized.
const CHIP_CHARS: i32 = 28;
/// The class every coloured chip has besides its colour's.
const COLORED: &str = "sw-label-colored";

thread_local! {
    static COLORS: RefCell<BTreeMap<String, AccountColor>> = RefCell::default();
}

/// Takes `config.toml`'s label colours; true when they changed, so the chips shown should be
/// filled again.
pub fn set_colors(colors: &BTreeMap<String, AccountColor>) -> bool {
    COLORS.with(|current| {
        let mut current = current.borrow_mut();
        let changed = *current != *colors;
        if changed {
            *current = colors.clone();
        }
        changed
    })
}

/// The colour `label` shows in, if `[labels]` gives it one.
pub fn color(label: &str) -> Option<AccountColor> {
    COLORS.with(|colors| colors.borrow().get(label).copied())
}

fn class(color: AccountColor) -> String {
    format!("sw-label-{}", color.name())
}

/// Sets `chip` to show `text` in `color`, changing only what differs (the list binds rows
/// often).
fn style(chip: &gtk::Label, text: &str, color: Option<AccountColor>) {
    if chip.text().as_str() != text {
        chip.set_text(text);
    }
    for other in AccountColor::ALL {
        let wanted = Some(other) == color;
        let name = class(other);
        if chip.has_css_class(&name) != wanted {
            if wanted {
                chip.add_css_class(&name);
            } else {
                chip.remove_css_class(&name);
            }
        }
    }
    if chip.has_css_class(COLORED) != color.is_some() {
        if color.is_some() {
            chip.add_css_class(COLORED);
        } else {
            chip.remove_css_class(COLORED);
        }
    }
}

/// Fills `chips` with one chip per label, reusing the chips it holds: at most `most` labels,
/// then one "+N" chip for the rest. `chips` hides without any.
pub fn fill(chips: &gtk::Box, labels: &[String], most: usize) {
    let shown = labels.len().min(most);
    let rest = labels.len() - shown;
    let wanted = shown + usize::from(rest > 0);
    let mut existing = Vec::new();
    let mut child = chips.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        existing.push(widget);
    }
    for extra in existing.drain(wanted.min(existing.len())..) {
        chips.remove(&extra);
    }
    while existing.len() < wanted {
        let chip = gtk::Label::builder()
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .single_line_mode(true)
            .max_width_chars(CHIP_CHARS)
            .valign(gtk::Align::Center)
            .css_classes(["sw-label-chip"])
            .build();
        chips.append(&chip);
        existing.push(chip.upcast());
    }
    for (widget, label) in existing.iter().zip(labels) {
        if let Some(chip) = widget.downcast_ref::<gtk::Label>() {
            style(chip, label, color(label));
        }
    }
    if rest > 0
        && let Some(chip) = existing.last().and_then(|w| w.downcast_ref::<gtk::Label>())
    {
        style(chip, &format!("+{rest}"), None);
    }
    if chips.is_visible() == (wanted == 0) {
        chips.set_visible(wanted > 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_are_set_by_name() {
        let colors = BTreeMap::from([("Work".to_owned(), AccountColor::Teal)]);
        assert!(set_colors(&colors));
        assert!(!set_colors(&colors));
        assert_eq!(color("Work"), Some(AccountColor::Teal));
        assert_eq!(color("work"), None);
        assert_eq!(color("Reise"), None);
        assert_eq!(class(AccountColor::Teal), "sw-label-teal");
        assert!(set_colors(&BTreeMap::new()));
        assert_eq!(color("Work"), None);
    }
}
