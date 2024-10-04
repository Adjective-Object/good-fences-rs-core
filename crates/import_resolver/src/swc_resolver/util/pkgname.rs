// Extracts the package name from a package import specifier
pub fn package_name(import_specifier: &str) -> Option<&str> {
    if import_specifier.is_empty() {
        return None;
    }
    let idx = import_specifier.find('/').unwrap_or(import_specifier.len());
    let first_slash = &import_specifier[..idx];

    if import_specifier.starts_with('@') {
        if idx + 1 >= import_specifier.len() {
            return None;
        }
        match import_specifier[idx + 1..].find('/') {
            Some(tail_len) => Some(&import_specifier[..idx + tail_len + 1]),
            None => Some(import_specifier),
        }
    } else if first_slash == "." || first_slash == ".." {
        return None;
    } else {
        return Some(first_slash);
    }
}

fn prefix_dotslash(import_specifier: &str) -> String {
    if import_specifier.starts_with("./") {
        import_specifier.to_string()
    } else {
        format!("./{}", import_specifier)
    }
}

// Extracts the package name from a package import specifier
pub fn split_package_import(import_specifier: &str) -> Option<(&str, String)> {
    match package_name(import_specifier) {
        Some(pkg) => {
            let idx = pkg.len();
            let rest_str = &import_specifier[idx..];

            if rest_str.starts_with('/') {
                let mut s: String = String::with_capacity(1 + rest_str.len());
                s.push('.');
                s.push_str(rest_str);
                Some((pkg, s))
            } else if rest_str.is_empty() {
                return Some((pkg, ".".to_string()));
            } else if rest_str.starts_with("./") {
                return Some((pkg, rest_str.to_string()));
            } else {
                return Some((pkg, prefix_dotslash(rest_str)));
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
        assert_eq!(package_name("react-dom"), Some("react-dom"));
        assert_eq!(package_name("@react"), None);
        assert_eq!(package_name("@react/react-dom"), Some("@react/react-dom"));
        assert_eq!(
            package_name("@react/react-dom/server"),
            Some("@react/react-dom")
        );
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
            Some(("react-dom", ".".to_string()))
        );
        assert_eq!(split_package_import("@react"), None);
        assert_eq!(
            split_package_import("@react/react-dom"),
            Some(("@react/react-dom", ".".to_string()))
        );
        assert_eq!(
            split_package_import("@react/react-dom/server"),
            Some(("@react/react-dom", "./server".to_string()))
        );
    }
}
