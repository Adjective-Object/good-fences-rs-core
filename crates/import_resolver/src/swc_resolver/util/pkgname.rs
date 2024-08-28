// Extracts the package name from a package import specifier
pub fn package_name(import_specifier: &str) -> Option<&str> {
    let idx = import_specifier
        .find('/')
        .unwrap_or_else(|| import_specifier.len());
    let first_slash = &import_specifier[..idx];
    if import_specifier.starts_with('@') {
        return first_slash
            .find('/')
            .map(|idx2| &import_specifier[..idx + idx2]);
    } else {
        return Some(first_slash);
    }
}

// Extracts the package name from a package import specifier
pub fn split_package_import(import_specifier: &str) -> Option<(&str, String)> {
    match package_name(import_specifier) {
        Some(pkg) => {
            let idx = pkg.len();
            let rest_str = &import_specifier[idx..];

            if rest_str.starts_with('/') {
                let mut s = String::with_capacity(1 + rest_str.len());
                s.push('.');
                s.push_str(rest_str);
                return Some((pkg, s));
            } else if !rest_str.starts_with("./") {
                let mut s = String::with_capacity(2 + rest_str.len());
                s.push_str("./");
                s.push_str(rest_str);
                return Some((pkg, s));
            } else {
                return Some((pkg, rest_str.to_string()));
            }
        }
        None => None,
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_package_name() {
        assert_eq!(package_name("./"), None);
        assert_eq!(package_name(""), None);
        assert_eq!(package_name("./src/index.ts"), None);
        assert_eq!(package_name("react"), Some("react"));
        assert_eq!(package_name("react-dom"), Some("react"));
        assert_eq!(package_name("@react"), None);
        assert_eq!(package_name("@react/react-dom"), Some("@react"));
        assert_eq!(package_name("@react/react-dom/server"), Some("@react"));
    }

    #[test]
    fn test_split_package_import() {
        assert_eq!(split_package_import("./"), None);
        assert_eq!(split_package_import(""), None);
        assert_eq!(split_package_import("./src/index.ts"), None);
        assert_eq!(
            split_package_import("react"),
            Some(("react", ".".to_string()))
        );
        assert_eq!(
            split_package_import("react-dom"),
            Some(("react", ".".to_string()))
        );
        assert_eq!(split_package_import("@react"), None);
        assert_eq!(
            split_package_import("@react/react-dom"),
            Some(("@react/react-dom", ".".to_string()))
        );
        assert_eq!(
            split_package_import("@react/react-dom/server"),
            Some(("@react/react=-dom", "/server".to_string()))
        );
    }
}
