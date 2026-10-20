//! HTML signatures (issue #6): a signature file the user designs, as the composer inserts it.
//! The app reads the file and the images it names beside it ([`image_sources`]); [`build`]
//! embeds the images as `data:` images (the composer and [`super::data_images`] make them parts
//! of the draft, sent by `cid:`), keeps what the allowlist below allows and wraps the rest in the
//! signature's `div` ([`super::clean::SIGNATURE_CLASS`]).
//!
//! The allowlist is the composer's own (S7's tags) with `span`, table layout attributes, `tel:`
//! links and a `style` attribute whose declarations pass [`style`]: colours, fonts, spacing,
//! borders and sizes, never `url()`, `expression`, comments, escapes or another function than
//! `rgb`/`hsl`. The composer's helper keeps the same inside the signature's `div` and nowhere
//! else (`app/src/composer/helper.js`, `signatureStyle`). Nothing loads: an image is a file
//! beside the signature or nothing; a remote one is refused.

use super::clean::SIGNATURE_CLASS;
use super::identity::Signature;
use super::mime;
use ammonia::{Builder, UrlRelative};
use mail_builder::encoders::base64::Base64Encoder;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex};

/// The largest signature file read.
pub const MAX_HTML_BYTES: u64 = 256 * 1024;
/// The largest image a signature embeds.
pub const MAX_IMAGE_BYTES: u64 = 1024 * 1024;
/// The most a signature's images may add up to.
pub const MAX_IMAGES_BYTES: u64 = 4 * 1024 * 1024;

/// Tags kept: the composer's (S7), `span` and `small`; `strong` and `em` stay as they are.
/// Others (`font`, `center`) go, their content kept: style a `span` instead.
const TAGS: [&str; 37] = [
    "a", "b", "strong", "i", "em", "u", "s", "strike", "del", "sub", "sup", "small", "br", "hr",
    "p", "div", "span", "table", "thead", "tbody", "tfoot", "tr", "td", "th", "caption", "img",
    "ul", "ol", "li", "h1", "h2", "h3", "h4", "h5", "h6", "pre", "code",
];
/// Removed with everything inside them, as the composer's cleaner removes them.
const DROP: [&str; 12] = [
    "script", "style", "iframe", "object", "embed", "form", "svg", "math", "template", "noscript",
    "head", "title",
];
/// CSS properties a signature's `style` keeps.
pub const PROPERTIES: [&str; 48] = [
    "color",
    "background",
    "background-color",
    "font",
    "font-family",
    "font-size",
    "font-weight",
    "font-style",
    "font-variant",
    "text-decoration",
    "text-align",
    "text-transform",
    "text-indent",
    "vertical-align",
    "line-height",
    "letter-spacing",
    "word-spacing",
    "white-space",
    "margin",
    "margin-top",
    "margin-right",
    "margin-bottom",
    "margin-left",
    "padding",
    "padding-top",
    "padding-right",
    "padding-bottom",
    "padding-left",
    "border",
    "border-top",
    "border-right",
    "border-bottom",
    "border-left",
    "border-color",
    "border-width",
    "border-style",
    "border-collapse",
    "border-spacing",
    "border-radius",
    "width",
    "min-width",
    "max-width",
    "height",
    "min-height",
    "max-height",
    "display",
    "table-layout",
    "list-style-type",
];
/// What `display` may be: no `none`, nothing that takes the signature out of the flow.
pub const DISPLAYS: [&str; 7] = [
    "inline",
    "block",
    "inline-block",
    "table",
    "table-row",
    "table-cell",
    "list-item",
];
/// The only CSS functions a value may call.
pub const FUNCTIONS: [&str; 4] = ["rgb", "rgba", "hsl", "hsla"];

/// Why a signature file can't be the signature. Its text says what to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// Nothing is left once the allowlist is through.
    Empty,
    /// An image on the web; only files beside the signature are embedded.
    RemoteImage(String),
    /// An image the file names that isn't beside it (or wasn't read).
    MissingImage(String),
    /// A file the signature names as an image that isn't PNG, JPEG, GIF or WebP.
    NotAnImage(String),
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Problem::Empty => f.write_str("the signature file shows nothing Sealwax can send"),
            Problem::RemoteImage(source) => write!(
                f,
                "the image {} is on the web; put the file beside the signature and name it \
                 there, like src=\"logo.png\"",
                shown(source)
            ),
            Problem::MissingImage(source) => write!(
                f,
                "the image {} isn't beside the signature file",
                shown(source)
            ),
            Problem::NotAnImage(source) => {
                write!(f, "{} isn't a PNG, JPEG, GIF or WebP image", shown(source))
            }
        }
    }
}

/// A source as an error quotes it: at most 80 characters.
fn shown(source: &str) -> String {
    let cut: String = source.chars().take(80).collect();
    if cut.len() < source.len() {
        format!("“{cut}…”")
    } else {
        format!("“{cut}”")
    }
}

/// What an image's source names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    /// A file beside the signature.
    Local,
    /// `http:`, `https:` or `//`: on the web.
    Remote,
    /// An image already inline (`data:` of a type every client shows).
    Inline,
    /// Anything else (`cid:`, `file:`, `javascript:`…): dropped.
    Other,
}

fn source(value: &str) -> Source {
    let lower = value.trim().to_ascii_lowercase();
    if lower.starts_with("http:") || lower.starts_with("https:") || lower.starts_with("//") {
        Source::Remote
    } else if ["png", "jpeg", "gif", "webp"]
        .iter()
        .any(|kind| lower.starts_with(&format!("data:image/{kind};base64,")))
    {
        Source::Inline
    } else if lower.is_empty()
        || lower
            .split_once(':')
            .is_some_and(|(scheme, _)| !scheme.contains('/') && !scheme.contains('.'))
    {
        Source::Other
    } else {
        Source::Local
    }
}

/// The files a signature names as images, each once, in order of appearance: the app reads
/// them beside the signature file and hands them to [`build`].
pub fn image_sources(html: &str) -> Vec<String> {
    let found = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = Arc::clone(&found);
    let mut builder = Builder::empty();
    builder
        .tags(HashSet::from(["img"]))
        .tag_attributes(HashMap::from([("img", HashSet::from(["src"]))]))
        .url_relative(UrlRelative::PassThrough)
        .attribute_filter(move |element, attribute, value| {
            if element == "img" && attribute == "src" && source(value) == Source::Local {
                let mut seen = seen.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                let value = value.trim().to_owned();
                if !seen.contains(&value) {
                    seen.push(value);
                }
            }
            None
        });
    let _ = builder.clean(html);
    let found = found
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    found.clone()
}

/// The image type of `bytes` by its first bytes: PNG, JPEG, GIF or WebP.
pub fn image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// `html`, a signature file, as the signature the composer inserts: each image it names from
/// `images` (by the source as [`image_sources`] gives it) embedded as a `data:` image, what the
/// allowlist keeps in the signature's `div`. `text` is the plain signature for
/// [`Signature::text`]; without one, the HTML's text.
pub fn build(
    html: &str,
    text: Option<&str>,
    images: &HashMap<String, Vec<u8>>,
) -> Result<Signature, Problem> {
    let mut inline = HashMap::new();
    for wanted in image_sources(html) {
        let bytes = images
            .get(&wanted)
            .ok_or_else(|| Problem::MissingImage(wanted.clone()))?;
        let kind = image_type(bytes).ok_or_else(|| Problem::NotAnImage(wanted.clone()))?;
        let encoded = Base64Encoder::new()
            .encode(bytes)
            .map_err(|_| Problem::NotAnImage(wanted.clone()))?;
        let encoded = String::from_utf8_lossy(&encoded).replace(['\r', '\n'], "");
        inline.insert(wanted, format!("data:{kind};base64,{encoded}"));
    }
    let problems = Arc::new(Mutex::new(Vec::new()));
    let cleaned = sanitize(html, Arc::new(inline), Arc::clone(&problems));
    let problems = problems
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(problem) = problems.first() {
        return Err(problem.clone());
    }
    let has_image = cleaned.contains("<img");
    if mime::plain_text(&cleaned).trim().is_empty() && !has_image {
        return Err(Problem::Empty);
    }
    let html = format!("<div class=\"{SIGNATURE_CLASS}\">{}</div>", cleaned.trim());
    let text = match text.map(str::trim).filter(|text| !text.is_empty()) {
        Some(text) => text.to_owned(),
        None => mime::plain_text(&html).trim().to_owned(),
    };
    Ok(Signature {
        html,
        text: format!("-- \n{text}"),
    })
}

/// The allowlist: `inline` maps a local image source to its `data:` image; images that are on
/// the web or not read go to `problems`.
fn sanitize(
    html: &str,
    inline: Arc<HashMap<String, String>>,
    problems: Arc<Mutex<Vec<Problem>>>,
) -> String {
    let attributes = |names: &[&'static str]| names.iter().copied().collect::<HashSet<_>>();
    let mut tag_attributes = HashMap::from([
        ("a", attributes(&["href"])),
        ("img", attributes(&["src", "alt", "width", "height"])),
        (
            "table",
            attributes(&[
                "width",
                "border",
                "cellpadding",
                "cellspacing",
                "align",
                "bgcolor",
            ]),
        ),
        (
            "td",
            attributes(&[
                "width", "height", "align", "valign", "bgcolor", "colspan", "rowspan",
            ]),
        ),
        (
            "th",
            attributes(&[
                "width", "height", "align", "valign", "bgcolor", "colspan", "rowspan",
            ]),
        ),
        ("tr", attributes(&["align", "valign", "bgcolor"])),
        ("p", attributes(&["align"])),
        ("div", attributes(&["align"])),
    ]);
    for tag in TAGS {
        tag_attributes.entry(tag).or_default().insert("style");
    }
    let mut builder = Builder::empty();
    builder
        .tags(TAGS.into_iter().collect())
        .clean_content_tags(DROP.into_iter().collect())
        .tag_attributes(tag_attributes)
        .generic_attributes(HashSet::new())
        .url_schemes(attributes(&["http", "https", "mailto", "tel", "data"]))
        .url_relative(UrlRelative::PassThrough)
        .link_rel(None)
        .strip_comments(true)
        .attribute_filter(move |element, attribute, value| {
            let value = value.trim();
            match (element, attribute) {
                (_, "style") => style(value).map(Cow::Owned),
                ("a", "href") => {
                    let lower = value.to_ascii_lowercase();
                    ["http://", "https://", "mailto:", "tel:"]
                        .iter()
                        .any(|scheme| lower.starts_with(scheme))
                        .then(|| Cow::Owned(value.to_owned()))
                }
                ("img", "src") => match source(value) {
                    Source::Local => {
                        let found = inline.get(value).cloned();
                        if found.is_none() {
                            let mut problems = problems
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner());
                            problems.push(Problem::MissingImage(value.to_owned()));
                        }
                        found.map(Cow::Owned)
                    }
                    Source::Inline => Some(Cow::Owned(value.to_owned())),
                    Source::Remote => {
                        let mut problems = problems
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        problems.push(Problem::RemoteImage(value.to_owned()));
                        None
                    }
                    Source::Other => None,
                },
                ("img", "alt") => Some(Cow::Owned(value.to_owned())),
                ("img" | "td" | "th", "width" | "height") | ("table", "width") => {
                    layout_value(value).then(|| Cow::Owned(value.to_owned()))
                }
                ("td" | "th", "colspan" | "rowspan") => ((1..=3).contains(&value.len())
                    && value.bytes().all(|b| b.is_ascii_digit()))
                .then(|| Cow::Owned(value.to_owned())),
                (_, "align" | "valign" | "bgcolor" | "border" | "cellpadding" | "cellspacing") => {
                    layout_value(value).then(|| Cow::Owned(value.to_owned()))
                }
                _ => None,
            }
        });
    without_bare_images(&builder.clean(html).to_string())
}

/// `html` (ammonia's output, whose attribute values escape `>`) without the images whose
/// source the allowlist took away: an image without one shows as broken.
fn without_bare_images(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(at) = rest.find("<img") {
        out.push_str(&rest[..at]);
        let end = rest[at..].find('>').map_or(rest.len(), |end| at + end + 1);
        let tag = &rest[at..end];
        if tag.contains(" src=\"") {
            out.push_str(tag);
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// A table attribute's or a size's value: a number, a percentage, a word or a `#` colour.
fn layout_value(value: &str) -> bool {
    (1..=32).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"#%.-".contains(&b))
}

/// The declarations of a `style` attribute that [`PROPERTIES`] allows with a safe value, written
/// again (`property: value; …`); `None` when none is left.
pub fn style(value: &str) -> Option<String> {
    let kept: Vec<String> = value
        .split(';')
        .filter_map(|declaration| {
            let (property, value) = declaration.split_once(':')?;
            let property = property.trim().to_ascii_lowercase();
            if !PROPERTIES.contains(&property.as_str()) {
                return None;
            }
            let value = css_value(&property, value)?;
            Some(format!("{property}: {value}"))
        })
        .collect();
    (!kept.is_empty()).then(|| kept.join("; "))
}

/// A declaration's value when it is safe for `property`: plain characters only, no `url`,
/// `expression`, comment or escape, no function but [`FUNCTIONS`]; `!important` dropped.
fn css_value(property: &str, value: &str) -> Option<String> {
    let mut value = value.trim();
    let lower = value.to_ascii_lowercase();
    if let Some(at) = lower.find("!important") {
        value = value[..at].trim_end();
    }
    if value.is_empty() || value.len() > 200 {
        return None;
    }
    if !value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b" #%.,-+'\"()/_".contains(&b))
    {
        return None;
    }
    let lower = value.to_ascii_lowercase();
    if ["url", "expression", "javascript", "/*", "image", "var("]
        .iter()
        .any(|bad| lower.contains(bad))
    {
        return None;
    }
    let mut rest = lower.as_str();
    while let Some(open) = rest.find('(') {
        let name = rest[..open]
            .rsplit(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
            .next()
            .unwrap_or("");
        if !FUNCTIONS.contains(&name) {
            return None;
        }
        rest = &rest[open + 1..];
    }
    if property == "display" && !DISPLAYS.contains(&lower.as_str()) {
        return None;
    }
    Some(value.to_owned())
}

#[cfg(test)]
mod tests;
