use anyhow::{anyhow, Ok, Result};
use ouroboros::self_referencing;
use std::{
    collections::HashMap,
    fmt::{Display, Write},
    str::FromStr,
};

pub struct Patch {
    // comments on why the patch is necessary
    comment_lines: Vec<String>,
    // md5 of the target file the patch is meant to apply to
    target_file_md5: Option<String>,
    // format verison of the patch
    patch_format_version: String,
    // the underlying set of hubnks, represented as a patch
    inner_patch: SelfOwnedDiffyPatch,
}

// diffy patches do not own their own memory, instead they reference
// a diff against some underlying byte array or string.
//
// This struct wraps the diffy patch with its data.
#[self_referencing]
struct SelfOwnedDiffyPatch {
    // Underlying string buffer for the patch
    patch_contents: String,
    #[borrows(patch_contents)]
    #[not_covariant]
    patch: diffy::Patch<'this, str>,
}

impl SelfOwnedDiffyPatch {
    pub fn from_hunk_str(hunk_str: &str) -> Result<SelfOwnedDiffyPatch> {
        SelfOwnedDiffyPatch::try_new(
            hunk_str.to_string(),
            for<'a> |diff_text: &'a String| -> Result<diffy::Patch<'a, str>, anyhow::Error> {
                diffy::Patch::from_str(diff_text).map_err(|e| -> anyhow::Error { anyhow!("{e}") })
            },
        )
    }

    #[cfg(test)]
    pub fn num_hunk_lines(&self) -> usize {
        self.with_patch(|patch| {
            patch
                .hunks()
                .iter()
                .fold(0, |num_lines, hunk| num_lines + hunk.lines().len())
        })
    }
}

impl Display for SelfOwnedDiffyPatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.borrow_patch_contents())
    }
}

impl Patch {
    fn parse_header(s: &str) -> Result<(HashMap<String, String>, Vec<String>, &str)> {
        // parse the header
        let mut header_values = HashMap::<String, String>::new();
        let mut comment_lines: Vec<String> = Vec::new();
        let lines_iter = s.lines();
        let mut head = 0;
        for (i, line) in lines_iter.enumerate() {
            // +1 here accounts for the newline character
            let line_len = line.len() + 1;
            let line: &str = line.trim();
            if let Some(line) = line.strip_prefix("#>") {
                // This a header key/pair line of format:
                // #> property: value
                let mut split = line.split(":");
                let name = match split.next() {
                    None => return Err(anyhow!("malformed patch header on line {i}")),
                    Some(v) => v.trim().to_string(),
                };

                match split.remainder() {
                    None => {} //noop
                    Some(v) => {
                        let value = v.trim().to_string();
                        header_values.insert(name, value);
                    }
                }
            } else if line.starts_with("#") {
                comment_lines.push(line.to_string());
            } else {
                // we have left header data
                return Ok((header_values, comment_lines, s.split_at(head).1));
            }

            head += line_len
        }

        Ok((header_values, comment_lines, ""))
    }

    pub fn apply(&self, target_bytes: &str) -> Result<String, diffy::ApplyError> {
        self.inner_patch
            .with_patch(|patch: &diffy::Patch<'_, str>| diffy::apply(target_bytes, patch))
    }
}

impl FromStr for Patch {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let (header_props, comment_lines, remainder) = Patch::parse_header(s).unwrap();
        let patch_format_version =
            match header_props.get("patch_format_version").map(String::as_str) {
                Some("1") | None => "1".to_string(),
                Some(p) => return Err(anyhow!("unrecognized patch format version {p}")),
            };
        let target_file_md5 = header_props.get("target_file_md5").cloned();

        // unidiff requires file names, but the patch sets only contain hunks. Use synthetic files here.
        let inner_patch = SelfOwnedDiffyPatch::from_hunk_str(remainder)?;

        Ok(Patch {
            comment_lines,
            target_file_md5,
            patch_format_version,
            inner_patch,
        })
    }
}

impl Display for Patch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // push patch comments
        for line in self.comment_lines.iter() {
            f.write_str(line)?;
            f.write_str("\n")?;
        }

        // push patch header metadata
        f.write_str("#> patch_format_version: ")?;
        f.write_str(&self.patch_format_version)?;
        f.write_char('\n')?;
        if let Some(target_file_md5) = &self.target_file_md5 {
            f.write_str("#> target_file_md5: ")?;
            f.write_str(target_file_md5)?;
            f.write_char('\n')?;
        }

        // write the patch contents
        write!(f, "{}", self.inner_patch)?;

        std::fmt::Result::Ok(())
    }
}

#[cfg(test)]
mod test {
    use std::str::FromStr;

    use pretty_assertions::{assert_eq, assert_str_eq};

    use super::Patch;

    #[test]
    fn test_parse_patch_real() {
        let patch = Patch::from_str(
            "#> foo: bar baz bing bing bang\n\
    #> target_file_md5: asdf\n\
    #> target_file_md5: 12345678\n\
    #> patch_format_version: 1\n\
    @@ -1 +1 @@\n\
    -banana original content\n\
    +banana patched content\n\
    ",
        )
        .unwrap();
        if patch.target_file_md5 != Some("12345678".to_string()) {
            panic!(
                "Target md5 should be the last value in the header, got {:?}",
                patch.target_file_md5,
            )
        }
        if patch.patch_format_version != "1" {
            panic!(
                "Patch version should match, got {}",
                patch.patch_format_version,
            )
        }
        let num_hunk_lines = patch.inner_patch.num_hunk_lines();
        if num_hunk_lines != 2 {
            panic!("Patch should be parsed to a single, 2-line hunk. Got {num_hunk_lines}")
        }
    }

    #[test]
    fn test_parse_patch_real_crlf() {
        let patch = Patch::from_str(
            "#> foo: bar baz bing bing bang\r\n\
                #> target_file_md5: asdf\r\n\
                #> target_file_md5: 12345678\r\n\
                #> patch_format_version: 1\r\n\
                @@ -1 +1 @@\r\n\
                -banana original content\r\n\
                +banana patched content\r\n",
        )
        .unwrap();
        if patch.target_file_md5 != Some("12345678".to_string()) {
            panic!(
                "Target md5 should be the last value in the header, got {:?}",
                patch.target_file_md5,
            )
        }
        if patch.patch_format_version != "1" {
            panic!(
                "Patch version should match, got {}",
                patch.patch_format_version,
            )
        }
        let num_hunk_lines = patch.inner_patch.num_hunk_lines();
        if num_hunk_lines != 2 {
            panic!("Patch should be parsed to a single, 2-line hunk. Got {num_hunk_lines}")
        }
    }

    #[test]
    fn test_parse_patch_header_comments() {
        let patch = Patch::from_str(
            r#"#> foo: bar baz bing bing bang
#> target_file_md5: asdf
#> target_file_md5: 12345678
#
#
# support comments as well
#
#
#
@@ -1 +1 @@
-lemon original content
+lemon patched content
"#,
        )
        .unwrap();
        if patch.target_file_md5 != Some("12345678".to_string()) {
            panic!(
                "Target md5 should be the last value in the header, got {:?}",
                patch.target_file_md5,
            )
        }
        let num_hunk_lines = patch.inner_patch.num_hunk_lines();
        if num_hunk_lines != 2 {
            panic!("Patch should be parsed to a single, 2-line hunk. Got {num_hunk_lines}")
        }
    }

    #[test]
    fn test_patch_parse_fail() {
        let patch = Patch::from_str(
            r#"#> patch_format_version: 1
@@ -1 +1 @@
a
"#,
        );

        let err = match patch {
            Err(e) => e,
            _ => panic!("expected patch parse err"),
        };

        let err_str = err.to_string();
        assert_str_eq!(err_str, "error parsing patch: unexpected line in hunk body",);
    }

    #[test]
    fn test_patch_serialize() {
        let patch = Patch::from_str(
            r#"#>   target_file_md5   : 1234
#> patch_format_version: 1
# some-comments
@@ -1 +1 @@
-abcd
+apple patched content
@@ -2 +2 @@
-defg
+hello!
"#,
        )
        .unwrap();

        assert_str_eq!(
            "# some-comments
#> patch_format_version: 1
#> target_file_md5: 1234
@@ -1 +1 @@
-abcd
+apple patched content
@@ -2 +2 @@
-defg
+hello!
",
            patch.to_string(),
            "patch should serialize to a normal form",
        )
    }

    #[test]
    fn test_load_save_patch_stable() {
        let input = r#"4,11 @@
 import { ChangeGate } from '@augloop/settings';
 import { makeItemPathKey, NotFoundError, splitItemPathKey } from '@augloop/session-cache';
 import { AddOperation, AnnotationState, DeleteOperation, SchemaObject, UpdateAnnotationMetaDataOperation, UpdateOperation } from '@augloop/types-core';
-import * as deepEqual from 'fast-deep-equal';
+import deepEqual from 'fast-deep-equal';
 import { BaseAnnotationProcessor } from './base-annotation-processor';
 import { setAnnotationStateToSent } from './session';
 import { SequencerSeedMode } from './syncmessage-sequencer';
-import * as uuidv4 from 'uuid/v4';
+import uuidv4 from 'uuid/v4';
 var StatefulAnnotationProcessor = /** @class */ (function (_super) {
     __extends(StatefulAnnotationProcessor, _super);
     function StatefulAnnotationProcessor(session) {
"#;

        let patch = Patch::from_str(input).unwrap();
        let output = patch.to_string();

        let mut expected_output = String::from("#> patch_format_version: 1\n");
        expected_output.push_str(input);

        assert_eq!(expected_output, output)
    }
}
