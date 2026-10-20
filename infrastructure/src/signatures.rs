//! HTML signatures from their files (issue #6): `signature_html` in `accounts.toml` names an HTML
//! file, relative to Sealwax's config folder or absolute; the images it names lie beside it.
//! Read each time a message is started, so a changed file applies to the next one; what the
//! draft holds by then stays as it is.
//!
//! An image is a path relative to the signature's folder, never absolute and without `..`;
//! once symlinks are resolved it must still be in that folder. Sizes are capped
//! (`signature_html::MAX_*`). The rest, the allowlist and the embedding, is
//! `domain::compose::signature_html`'s.

use sealwax_domain::account::{AccountSettings, HtmlSignature};
use sealwax_domain::compose::identity::Signature;
use sealwax_domain::compose::signature_html::{
    self, MAX_HTML_BYTES, MAX_IMAGE_BYTES, MAX_IMAGES_BYTES,
};
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

/// Fills in `loaded` of every HTML signature in `accounts` (accounts and identities), read from
/// files under `config_dir`. Each falls back to its plain signature when it can't be read.
pub fn load_all(config_dir: &Path, accounts: &mut [AccountSettings]) {
    for account in accounts {
        load_into(
            config_dir,
            &mut account.signature_html,
            account.signature.as_deref(),
        );
        for identity in &mut account.identities {
            load_into(
                config_dir,
                &mut identity.signature_html,
                identity.signature.as_deref(),
            );
        }
    }
}

fn load_into(config_dir: &Path, signature: &mut Option<HtmlSignature>, text: Option<&str>) {
    if let Some(signature) = signature {
        signature.loaded = Some(load(config_dir, &signature.path, text));
    }
}

/// The signature in the HTML file at `path` (relative to `config_dir`, or absolute), its
/// plain text `text` when given; else what went wrong, in words for the user.
pub fn load(config_dir: &Path, path: &str, text: Option<&str>) -> Result<Signature, String> {
    let file = config_dir.join(path);
    let fail = |why: String| format!("The HTML signature {path} can't be used: {why}.");
    let html =
        read_capped(&file, MAX_HTML_BYTES).map_err(|why| fail(why.message(MAX_HTML_BYTES)))?;
    let html = String::from_utf8_lossy(&html).into_owned();
    let folder = file
        .parent()
        .and_then(|folder| folder.canonicalize().ok())
        .ok_or_else(|| fail("its folder can't be read".to_owned()))?;
    let mut images = HashMap::new();
    let mut total = 0u64;
    for source in signature_html::image_sources(&html) {
        let Some(image) = image_path(&folder, &source) else {
            return Err(fail(format!(
                "the image “{source}” isn't in the signature's folder; name it by its path \
                 from there, like src=\"logo.png\""
            )));
        };
        let bytes = match read_capped(&image, MAX_IMAGE_BYTES) {
            Ok(bytes) => bytes,
            Err(Unreadable::Missing) => continue,
            Err(why) => {
                return Err(fail(format!(
                    "the image “{source}” {}",
                    why.message(MAX_IMAGE_BYTES)
                )));
            }
        };
        total += bytes.len() as u64;
        if total > MAX_IMAGES_BYTES {
            return Err(fail(format!(
                "its images are larger than {} MiB together",
                MAX_IMAGES_BYTES / 1024 / 1024
            )));
        }
        images.insert(source, bytes);
    }
    signature_html::build(&html, text, &images).map_err(|problem| fail(problem.to_string()))
}

/// The file `source` names in `folder` (canonical), when it stays inside it: a relative path
/// without `..`, its query or fragment dropped and `%XX` decoded; symlinks resolved and checked
/// again. A file that doesn't exist yet is still `Some` (the build reports it as missing).
fn image_path(folder: &Path, source: &str) -> Option<PathBuf> {
    let source = source.split(['?', '#']).next().unwrap_or_default();
    let decoded = percent_decode(source)?;
    let relative = Path::new(&decoded);
    if decoded.is_empty()
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
    {
        return None;
    }
    let joined = folder.join(relative);
    match joined.canonicalize() {
        Ok(real) => real.starts_with(folder).then_some(real),
        Err(_) => Some(joined),
    }
}

/// `%XX` decoded; `None` when the result isn't UTF-8.
fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && let Some(hex) = text.get(index + 1..index + 3)
            && let Ok(byte) = u8::from_str_radix(hex, 16)
        {
            out.push(byte);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Why a file wasn't read.
#[derive(Debug, PartialEq, Eq)]
enum Unreadable {
    Missing,
    NotAFile,
    TooLarge,
    Failed,
}

impl Unreadable {
    fn message(&self, limit: u64) -> String {
        match self {
            Unreadable::Missing => "isn't there".to_owned(),
            Unreadable::NotAFile => "isn't a file".to_owned(),
            Unreadable::TooLarge => format!("is larger than {} KiB", limit / 1024),
            Unreadable::Failed => "can't be read".to_owned(),
        }
    }
}

/// The bytes of the regular file at `path`, at most `limit` of them.
fn read_capped(path: &Path, limit: u64) -> Result<Vec<u8>, Unreadable> {
    let metadata = fs::metadata(path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => Unreadable::Missing,
        _ => Unreadable::Failed,
    })?;
    if !metadata.is_file() {
        return Err(Unreadable::NotAFile);
    }
    if metadata.len() > limit {
        return Err(Unreadable::TooLarge);
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    fs::File::open(path)
        .and_then(|file| file.take(limit + 1).read_to_end(&mut bytes))
        .map_err(|_| Unreadable::Failed)?;
    if bytes.len() as u64 > limit {
        return Err(Unreadable::TooLarge);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";

    fn folder() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("signatures/work/img")).unwrap();
        dir
    }

    fn write(dir: &Path, path: &str, bytes: &[u8]) {
        fs::write(dir.join(path), bytes).unwrap();
    }

    #[test]
    fn a_signature_and_its_images_beside_it() {
        let dir = folder();
        write(dir.path(), "signatures/work/img/logo.png", PNG);
        write(
            dir.path(),
            "signatures/work/signature.html",
            b"<p style=\"color: #333\">Ada</p><img src=\"img/logo.png\" alt=\"Logo\">\
              <img src=\"./img/logo%2Epng?v=2\">",
        );
        let signature = load(dir.path(), "signatures/work/signature.html", Some("Ada")).unwrap();
        assert!(signature.html.contains("<p style=\"color: #333\">Ada</p>"));
        assert_eq!(
            signature
                .html
                .matches("src=\"data:image/png;base64,")
                .count(),
            2
        );
        assert_eq!(signature.text, "-- \nAda");
        // An absolute path works as well.
        let absolute = dir.path().join("signatures/work/signature.html");
        assert!(load(Path::new("/nowhere"), absolute.to_str().unwrap(), None).is_ok());
    }

    #[test]
    fn images_outside_the_folder_are_refused() {
        let dir = folder();
        write(dir.path(), "secret.png", PNG);
        for source in ["../../secret.png", "/etc/passwd", "img/../../../secret.png"] {
            write(
                dir.path(),
                "signatures/work/signature.html",
                format!("Ada<img src=\"{source}\">").as_bytes(),
            );
            let error = load(dir.path(), "signatures/work/signature.html", None).unwrap_err();
            assert!(
                error.contains("isn't in the signature's folder"),
                "{source}: {error}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_out_of_the_folder_is_refused() {
        let dir = folder();
        write(dir.path(), "secret.png", PNG);
        std::os::unix::fs::symlink(
            dir.path().join("secret.png"),
            dir.path().join("signatures/work/img/logo.png"),
        )
        .unwrap();
        write(
            dir.path(),
            "signatures/work/signature.html",
            b"Ada<img src=\"img/logo.png\">",
        );
        let error = load(dir.path(), "signatures/work/signature.html", None).unwrap_err();
        assert!(error.contains("isn't in the signature's folder"), "{error}");
    }

    #[test]
    fn missing_and_oversized_files_say_so() {
        let dir = folder();
        assert_eq!(
            load(dir.path(), "signatures/work/none.html", None).unwrap_err(),
            "The HTML signature signatures/work/none.html can't be used: isn't there."
        );
        write(
            dir.path(),
            "signatures/work/signature.html",
            b"Ada<img src=\"logo.png\">",
        );
        let missing = load(dir.path(), "signatures/work/signature.html", None).unwrap_err();
        assert!(missing.ends_with("the image “logo.png” isn't beside the signature file."));
        let mut big = PNG.to_vec();
        big.resize(MAX_IMAGE_BYTES as usize + 1, 0);
        write(dir.path(), "signatures/work/logo.png", &big);
        let large = load(dir.path(), "signatures/work/signature.html", None).unwrap_err();
        assert!(large.contains("is larger than 1024 KiB"), "{large}");
        assert!(
            load(dir.path(), "signatures/work", None)
                .unwrap_err()
                .ends_with("isn't a file.")
        );
    }

    #[test]
    fn every_account_and_identity_is_read() {
        let dir = folder();
        write(dir.path(), "signatures/work/signature.html", b"<b>Ada</b>");
        let text = "[[account]]\nkey = \"work\"\nname = \"Ada\"\naddress = \"ada@example.test\"\n\
                    signature = \"Ada (plain)\"\n\
                    signature_html = \"signatures/work/signature.html\"\n\
                    [account.imap]\nhost = \"imap.example.test\"\n\
                    [account.smtp]\nhost = \"smtp.example.test\"\n\
                    [[account.identity]]\naddress = \"board@example.test\"\n\
                    signature = \"Board\"\nsignature_html = \"signatures/board.html\"\n";
        let mut accounts = sealwax_domain::accounts_file::parse(text).unwrap();
        load_all(dir.path(), &mut accounts);
        let account = &accounts[0];
        let own = account.signature_html.as_ref().unwrap().loaded.clone();
        assert!(own.unwrap().unwrap().html.contains("<b>Ada</b>"));
        let board = &account.identities[0];
        assert!(matches!(
            board.signature_html.as_ref().unwrap().loaded,
            Some(Err(_))
        ));
        // The board's file is missing: its mail carries the plain signature.
        let shown = sealwax_domain::compose::identity::signature_of(board).unwrap();
        assert!(shown.html.contains("Board"));
    }
}
