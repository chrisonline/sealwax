use super::*;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";

fn images(entries: &[(&str, &[u8])]) -> HashMap<String, Vec<u8>> {
    entries
        .iter()
        .map(|(source, bytes)| ((*source).to_owned(), bytes.to_vec()))
        .collect()
}

fn built(html: &str) -> String {
    build(html, None, &HashMap::new()).unwrap().html
}

#[test]
fn styles_keep_what_a_signature_needs() {
    assert_eq!(
        style("color: #1a5c2e; font-size: 12px; font-family: 'Segoe UI', Arial, sans-serif"),
        Some("color: #1a5c2e; font-size: 12px; font-family: 'Segoe UI', Arial, sans-serif".into())
    );
    assert_eq!(
        style("COLOR: rgb(10, 20, 30) !important; padding:0 0 4px 0;"),
        Some("color: rgb(10, 20, 30); padding: 0 0 4px 0".into())
    );
    assert_eq!(
        style("border-left: 2px solid hsla(120, 50%, 40%, 0.5); font: 12px/1.4 Arial"),
        Some("border-left: 2px solid hsla(120, 50%, 40%, 0.5); font: 12px/1.4 Arial".into())
    );
    assert_eq!(
        style("display: inline-block"),
        Some("display: inline-block".into())
    );
}

#[test]
fn styles_drop_what_could_load_hide_or_escape() {
    for refused in [
        "background: url(https://tracker.example.test/p.gif)",
        "background-image: url(logo.png)",
        "width: expression(alert(1))",
        "color: var(--x)",
        "font-family: \\61 rial",
        "color: red /* x */",
        "position: absolute",
        "display: none",
        "behavior: url(x.htc)",
        "width: calc(100% - 2px)",
        "color: <b>",
        "color: ",
        "garbage",
    ] {
        assert_eq!(style(refused), None, "{refused}");
    }
    assert_eq!(
        style("position: fixed; color: red; top: 0"),
        Some("color: red".into())
    );
}

#[test]
fn a_company_signature_keeps_its_layout() {
    let html = r#"<html><head><style>p{color:red}</style><title>Sig</title></head><body>
        <table cellpadding="0" cellspacing="0" border="0" style="font-family: Arial; color: #333">
          <tr><td valign="top" style="padding-right: 12px; border-right: 2px solid #1a5c2e">
            <img src="logo.png" width="96" height="40" alt="Grasser IT"></td>
          <td><span style="font-size: 14px; font-weight: bold">Christian Grasser</span><br>
            <a href="https://example.test" style="color: #1a5c2e">example.test</a> ·
            <a href="tel:+43123456">+43 123 456</a> · <a href="mailto:info@example.test">Mail</a>
          </td></tr></table></body></html>"#;
    let signature = build(html, None, &images(&[("logo.png", PNG)])).unwrap();
    let out = &signature.html;
    assert!(
        out.starts_with("<div class=\"sw-signature\"><table"),
        "{out}"
    );
    assert!(out.ends_with("</table></div>"), "{out}");
    for kept in [
        "cellpadding=\"0\"",
        "cellspacing=\"0\"",
        "border=\"0\"",
        "style=\"font-family: Arial; color: #333\"",
        "valign=\"top\"",
        "style=\"padding-right: 12px; border-right: 2px solid #1a5c2e\"",
        "src=\"data:image/png;base64,iVBORw0KGgoAAAANSUhEUg==\"",
        "width=\"96\"",
        "alt=\"Grasser IT\"",
        "<span style=\"font-size: 14px; font-weight: bold\">Christian Grasser</span>",
        "href=\"https://example.test\"",
        "href=\"tel:+43123456\"",
        "href=\"mailto:info@example.test\"",
    ] {
        assert!(out.contains(kept), "{kept} not in {out}");
    }
    for gone in ["<style", "color:red", "<title", "Sig</", "<html", "<body"] {
        assert!(!out.contains(gone), "{gone} in {out}");
    }
    assert!(signature.text.starts_with("-- \n"));
    assert!(signature.text.contains("Christian Grasser"));
}

#[test]
fn scripts_handlers_and_odd_links_go() {
    let out = built(
        "<div onclick=\"steal()\" style=\"color: red\">Ada<script>steal()</script>\
         <a href=\"javascript:steal()\">x</a><a href=\"ftp://example.test\">y</a>\
         <iframe src=\"https://example.test\"></iframe><font color=\"red\">z</font>\
         <form><input value=\"1\"></form><svg><circle/></svg></div>",
    );
    assert_eq!(
        out,
        "<div class=\"sw-signature\"><div style=\"color: red\">Ada<a>x</a><a>y</a>z</div></div>"
    );
}

#[test]
fn images_are_files_beside_the_signature_and_nothing_else() {
    assert_eq!(
        build("<img src=\"logo.png\">", None, &HashMap::new()),
        Err(Problem::MissingImage("logo.png".into()))
    );
    assert_eq!(
        build(
            "<img src=\"https://example.test/logo.png\">Ada",
            None,
            &HashMap::new()
        ),
        Err(Problem::RemoteImage("https://example.test/logo.png".into()))
    );
    assert_eq!(
        build(
            "<img src=\"//cdn.example.test/l.png\">Ada",
            None,
            &HashMap::new()
        ),
        Err(Problem::RemoteImage("//cdn.example.test/l.png".into()))
    );
    assert_eq!(
        build(
            "<img src=\"logo.png\">",
            None,
            &images(&[("logo.png", b"<svg onload=x>")])
        ),
        Err(Problem::NotAnImage("logo.png".into()))
    );
    // cid: and file: name nothing Sealwax has: left out, the text stays.
    assert_eq!(
        built("<img src=\"cid:logo@x\"><img src=\"file:///etc/passwd\">Ada"),
        "<div class=\"sw-signature\">Ada</div>"
    );
    // An image already inline stays inline.
    let inline = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUg==";
    assert_eq!(
        built(&format!("<img src=\"{inline}\">")),
        format!("<div class=\"sw-signature\"><img src=\"{inline}\"></div>")
    );
}

#[test]
fn the_plain_text_is_the_plain_signature_or_the_htmls_text() {
    let html = "<p><b>Ada Lovelace</b></p><p>Analytical Engines</p>";
    let own = build(html, Some("  Ada\nEngines \n"), &HashMap::new()).unwrap();
    assert_eq!(own.text, "-- \nAda\nEngines");
    let derived = build(html, None, &HashMap::new()).unwrap();
    assert!(derived.text.starts_with("-- \n"));
    assert!(derived.text.contains("Ada Lovelace"));
    assert!(derived.text.contains("Analytical Engines"));
}

#[test]
fn a_signature_shows_something() {
    for empty in [
        "",
        "   ",
        "<script>x</script>",
        "<p><br></p>",
        "<!-- note -->",
    ] {
        assert_eq!(
            build(empty, None, &HashMap::new()),
            Err(Problem::Empty),
            "{empty}"
        );
    }
    let logo_only = build("<img src=\"l.png\">", None, &images(&[("l.png", PNG)]));
    assert!(logo_only.is_ok());
}

#[test]
fn image_sources_lists_the_local_files_once() {
    assert_eq!(
        image_sources(
            "<img src=\" logo.png \"><img src=\"img/badge.gif\"><img src=\"logo.png\">\
             <img src=\"https://example.test/x.png\"><img src=\"data:image/png;base64,AA==\">\
             <img src=\"cid:a@b\"><script><img src=\"hidden.png\"></script>\
             <a href=\"page.html\">x</a>"
        ),
        ["logo.png", "img/badge.gif"]
    );
}

#[test]
fn image_types_by_their_first_bytes() {
    assert_eq!(image_type(PNG), Some("image/png"));
    assert_eq!(image_type(b"\xff\xd8\xff\xe0rest"), Some("image/jpeg"));
    assert_eq!(image_type(b"GIF89a..."), Some("image/gif"));
    assert_eq!(image_type(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
    assert_eq!(image_type(b"<svg>"), None);
    assert_eq!(image_type(b""), None);
}

#[test]
fn problems_say_what_to_do() {
    assert_eq!(
        Problem::RemoteImage("https://example.test/logo.png".into()).to_string(),
        "the image “https://example.test/logo.png” is on the web; put the file beside the \
         signature and name it there, like src=\"logo.png\""
    );
    assert_eq!(
        Problem::MissingImage("logo.png".into()).to_string(),
        "the image “logo.png” isn't beside the signature file"
    );
    let long = "x".repeat(100);
    assert!(Problem::NotAnImage(long).to_string().contains("…”"));
}
