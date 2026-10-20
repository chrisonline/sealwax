# Vendored crates

## lettre 0.11.23, with a longer reply-line limit

`vendor/lettre/` is [lettre](https://crates.io/crates/lettre) 0.11.23 exactly as published on crates.io (MIT, `LICENSE`), with one change: `MAX_RESPONSE_LINE_BYTES` in `src/transport/smtp/client/mod.rs` is 16,384 bytes instead of 1,000, with its comment. The 100,000-byte cap on a whole reply is unchanged.

Why: Microsoft 365 can answer the end of a message with a reply line longer than 1,000 bytes, which lettre can't read, so Sealwax couldn't tell whether the message was sent ([issue #2](https://github.com/PixDevsApps/sealwax/issues/2)). The limit isn't configurable in lettre; the upstream request is [lettre/lettre#1168](https://github.com/lettre/lettre/issues/1168).

The workspace's `Cargo.toml` and `fuzz/Cargo.toml` use it through `[patch.crates-io]`; it is excluded from the workspace, so `cargo fmt`, Clippy and the tests don't run on it.

A build prints four dead-code warnings from lettre: Cargo shows warnings for a crate in a local folder, which it hides for one from crates.io. They are lettre's own, from features Sealwax leaves off.

To check that nothing else differs, compare it with the published crate, a gzipped tarball whose SHA-256 is the checksum the lock file had before the patch (`f2c646bd5cc763b1087b15493e29a64be6147ba8f19342004fa52048ee596eae`):

```bash
mkdir -p target/lettre-check && curl -sSL https://static.crates.io/crates/lettre/lettre-0.11.23.crate -o target/lettre-check/lettre-0.11.23.crate
```

```bash
sha256sum target/lettre-check/lettre-0.11.23.crate && tar -xzf target/lettre-check/lettre-0.11.23.crate -C target/lettre-check
```

```bash
diff -r target/lettre-check/lettre-0.11.23 vendor/lettre
```

The only difference should be `src/transport/smtp/client/mod.rs`'s constant and its comment.

To drop it once lettre releases a fix: remove both `[patch.crates-io]` entries and this folder, raise `lettre` in `engine/Cargo.toml` (and `spikes/s5-oauth/Cargo.toml`), update the lock files, and keep `engine/src/smtp/submit/tests.rs`'s long-reply cases passing. To move it to a newer lettre, copy the new published crate here and make the same one-line change.

## imap-proto 0.17.0, reading Proton Mail Bridge's replies

`vendor/imap-proto/` is [imap-proto](https://crates.io/crates/imap-proto) 0.17.0 exactly as published on crates.io (MIT or Apache-2.0, `LICENSE-MIT`, `LICENSE-APACHE`), with three changes, each with a test:

- `body_param` takes an empty list, `()`, as no parameters, as it takes `NIL` (`test_body_param_empty_list_is_none`);
- `body_type_message` takes a `message/rfc822` part without the space between its envelope and its body (`test_body_structure_message_without_space_after_envelope`);
- `quoted` (`src/core.rs`) keeps a backslash followed by any other TEXT-CHAR as it is, where the grammar escapes only `\` and `"` (`test_quoted`, whose three "not allowed escapes" now pass through, and a line break after `\` is still refused).

The first two are in `src/rfc3501/body_structure.rs`.

Why: Proton Mail Bridge breaks RFC 3501's grammar three ways in BODYSTRUCTURE. It describes a part whose `Content-Disposition` is `inline` without parameters as `("inline" ())`, where the grammar allows only `NIL` or a non-empty list; it writes a forwarded message's part (`message/rfc822`) as `…"<message-id>")("text" …`, without the space the grammar requires after the envelope; and it escapes characters in file names as JSON does (`"Bildschirm\u00adfoto….png"`, a soft hyphen), where a quoted string may escape only `\` and `"`. imap-proto refused the whole FETCH reply either way; async-imap turned that into an i/o error that broke the connection, and every account pass that reached such a message failed there ("the command connection was lost", i/o error). Folders after it in the list never finished their first sync: on one Proton account (2026-10-20) Archive, Trash and the larger label folders stayed unfinished, and no pass reached the folders after them. The crate already accepts iCloud's `NIL` encoding the same way. `engine/src/imap/fetch.rs`'s `bridges_empty_disposition_parameters_parse` parses Bridge's first kind of reply as it came.

The workspace's `Cargo.toml` and `fuzz/Cargo.toml` use it through `[patch.crates-io]`; it is excluded from the workspace, so `cargo fmt`, Clippy and the tests don't run on it (`cargo test --lib` in its folder does).

To check that nothing else differs, compare it with the published crate, whose SHA-256 is the checksum the lock file had before the patch (`5ccf963d57074747b455398a1763d174da80bcaba6f51e3671a82252b531a68b`):

```bash
mkdir -p target/imap-proto-check && curl -sSL https://static.crates.io/crates/imap-proto/imap-proto-0.17.0.crate -o target/imap-proto-check/imap-proto-0.17.0.crate
```

```bash
sha256sum target/imap-proto-check/imap-proto-0.17.0.crate && tar -xzf target/imap-proto-check/imap-proto-0.17.0.crate -C target/imap-proto-check
```

```bash
diff -r target/imap-proto-check/imap-proto-0.17.0 vendor/imap-proto
```

The only differences should be `body_param`'s new alternative, `body_type_message`'s optional space and `quoted`'s escape (with `one_of` no longer imported and `verify` imported), each with its comment, the two new tests and `test_quoted`'s changed lines.

To drop it once imap-proto accepts all three: remove both `[patch.crates-io]` entries and this folder, update the lock files, and keep `bridges_empty_disposition_parameters_parse` passing.
