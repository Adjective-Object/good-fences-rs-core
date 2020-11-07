
use std::option::Option;
use std::path::Path;

// custom extension getter because
// we want to consider all dots in the filename so that
// `.d.ts` is one big extension.
pub fn get_extension_from_filename(filename: &str) -> Option<String> {
    if filename.ends_with(".d.ts") {
        return Some("d.ts".to_owned());
    }
    //Change it to a canonical file path.
    let path = Path::new(&filename);
    return match path.extension() {
        Some(ext) => Some(ext.to_owned().into_string().unwrap()),
        None => None,
    };
}

pub fn no_ext<'a>(s: &'a str) -> &'a str {
    let ext_opt = get_extension_from_filename(s);
    match ext_opt {
        Some(ext) => {
            return &s[0..s.len() - ext.len() - 1];
        }
        None => {
            return s;
        }
    }
}

#[cfg(test)]
mod test {
    use crate::file_extension::{get_extension_from_filename, no_ext};

    #[test]
    fn test_get_extension_from_filename_dts_ext() {
        let x = get_extension_from_filename("blah/foo.d.ts");
        assert_eq!(x, Some("d.ts".to_owned()));
    }

    #[test]
    fn test_get_extension_from_filename_spec_ts_ext() {
        let x = get_extension_from_filename("blah/foo.spec.ts");
        assert_eq!(x, Some("ts".to_owned()));
    }

    #[test]
    fn test_get_extension_from_filename_ts_ext() {
        let x = get_extension_from_filename("blah/foo.ts");
        assert_eq!(x, Some("ts".to_owned()));
    }

    #[test]
    fn test_get_extension_from_filename_no_ext() {
        let x = get_extension_from_filename("blah/foo");
        assert_eq!(x, None);
    }

    #[test]
    fn test_no_ext_dts_spec_ts_ext() {
        let x = get_extension_from_filename("blah/foo.spec.ts");
        assert_eq!(x, Some("ts".to_owned()));
    }

    #[test]
    fn test_no_ext_dts_ext() {
        let x = no_ext("blah/foo.d.ts");
        assert_eq!(x, "blah/foo");
    }

    #[test]
    fn test_no_ext_ts_ext() {
        let x = no_ext("blah/foo.ts");
        assert_eq!(x, "blah/foo");
    }

    #[test]
    fn test_no_ext_no_ext() {
        let x = no_ext("blah/foo");
        assert_eq!(x, "blah/foo")
    }
}
