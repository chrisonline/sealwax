//! Images inline in a draft's HTML as `data:` (an HTML signature's, [`super::signature_html`]),
//! made parts of the draft before it is stored: a draft that is sent without the composer (the
//! quick reply) still sends them as inline parts by `cid:`, as the composer does with the images
//! it holds. Sealwax writes these attributes itself (ammonia and the composer quote them with
//! `"`), so a scan for `src="data:image/…;base64,…"` finds them all.

use super::signature_html::image_type;
use mail_parser::decoders::base64::base64_decode;
use std::fmt;

/// One `data:` image taken out of the HTML. Its `Debug` prints sizes only.
#[derive(Clone, PartialEq, Eq)]
pub struct DataImage {
    /// Its Content-ID, without angle brackets: the HTML shows it as `cid:` of this.
    pub content_id: String,
    pub mime_type: String,
    pub data: Vec<u8>,
}

impl fmt::Debug for DataImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DataImage")
            .field("mime_type", &self.mime_type)
            .field("size", &self.data.len())
            .finish()
    }
}

const START: &str = "src=\"data:image/";

/// `html` with each `data:` image in a type every client shows (PNG, JPEG, GIF, WebP, the
/// bytes matching the type) shown by `cid:` of a Content-ID from `content_id`, and the images
/// in order; the same image twice is one part. Anything else is left as it is.
pub fn extract(
    html: &str,
    mut content_id: impl FnMut() -> Option<String>,
) -> (String, Vec<DataImage>) {
    let mut out = String::with_capacity(html.len());
    let mut images: Vec<(String, DataImage)> = Vec::new();
    let mut rest = html;
    while let Some(at) = rest.find(START) {
        let value_start = at + "src=\"".len();
        let Some(length) = rest[value_start..].find('"') else {
            break;
        };
        let value = &rest[value_start..value_start + length];
        out.push_str(&rest[..value_start]);
        let replaced = match images.iter().find(|(known, _)| known == value) {
            Some((_, image)) => Some(image.content_id.clone()),
            None => decode(value).and_then(|(mime_type, data)| {
                let id = content_id()?;
                images.push((
                    value.to_owned(),
                    DataImage {
                        content_id: id.clone(),
                        mime_type,
                        data,
                    },
                ));
                Some(id)
            }),
        };
        match replaced {
            Some(id) => {
                out.push_str("cid:");
                out.push_str(&id);
            }
            None => out.push_str(value),
        }
        rest = &rest[value_start + length..];
    }
    out.push_str(rest);
    (out, images.into_iter().map(|(_, image)| image).collect())
}

/// A `data:` image's type and bytes, when it is one every client shows.
fn decode(value: &str) -> Option<(String, Vec<u8>)> {
    let rest = value.strip_prefix("data:")?;
    let (mime_type, data) = rest.split_once(";base64,")?;
    let mime_type = mime_type.to_ascii_lowercase();
    let compact: String = data.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    let bytes = base64_decode(compact.as_bytes())?;
    (image_type(&bytes) == Some(mime_type.as_str())).then_some((mime_type, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";

    fn data_url(mime: &str, bytes: &[u8]) -> String {
        let encoded = mail_builder::encoders::base64::Base64Encoder::new()
            .encode(bytes)
            .unwrap();
        format!("data:{mime};base64,{}", String::from_utf8(encoded).unwrap())
    }

    #[test]
    fn data_images_become_parts_once() {
        let png = data_url("image/png", PNG);
        let html = format!(
            "<p>Hi</p><img src=\"{png}\" alt=\"Logo\"><img alt=\"again\" src=\"{png}\">\
             <img src=\"cid:old@x\">"
        );
        let mut next = 0;
        let (out, images) = extract(&html, || {
            next += 1;
            Some(format!("sig{next}@example.test"))
        });
        assert_eq!(
            out,
            "<p>Hi</p><img src=\"cid:sig1@example.test\" alt=\"Logo\">\
             <img alt=\"again\" src=\"cid:sig1@example.test\"><img src=\"cid:old@x\">"
        );
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].content_id, "sig1@example.test");
        assert_eq!(images[0].mime_type, "image/png");
        assert_eq!(images[0].data, PNG);
        assert!(!format!("{images:?}").contains("PNG"));
    }

    #[test]
    fn what_isnt_a_true_image_stays() {
        // Claims PNG, holds text; not base64; an SVG; no Content-ID to give.
        let fake = data_url("image/png", b"<script>");
        for html in [
            format!("<img src=\"{fake}\">"),
            "<img src=\"data:image/png;base64,***\">".to_owned(),
            "<img src=\"data:image/svg+xml;base64,PHN2Zz4=\">".to_owned(),
        ] {
            let (out, images) = extract(&html, || Some("x@y".into()));
            assert_eq!(out, html);
            assert!(images.is_empty());
        }
        let png = format!("<img src=\"{}\">", data_url("image/png", PNG));
        let (out, images) = extract(&png, || None);
        assert_eq!(out, png);
        assert!(images.is_empty());
        let (out, _) = extract("<img src=\"data:image/png", || Some("x@y".into()));
        assert_eq!(out, "<img src=\"data:image/png");
    }
}
