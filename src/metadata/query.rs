use crate::{
    env::MethodKind,
    error::{Error, Result},
};

use super::JavaMethodMetadata;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MethodQuery {
    pub(crate) class_pattern: String,
    pub(crate) method_pattern: String,
    pub(crate) include_signature: bool,
    pub(crate) ignore_case: bool,
    pub(crate) skip_system_classes: bool,
}

pub(crate) fn parse_method_query(query: &str) -> Result<MethodQuery> {
    let Some((class_pattern, rest)) = query.split_once('!') else {
        return Err(Error::InvalidQuery {
            query: query.to_owned(),
            message: "expected class!method query",
        });
    };
    if class_pattern.is_empty() {
        return Err(Error::InvalidQuery {
            query: query.to_owned(),
            message: "class pattern cannot be empty",
        });
    }

    let (method_pattern, modifiers) = if let Some((method, modifiers)) = rest.rsplit_once('/') {
        if modifiers.chars().all(|ch| matches!(ch, 'i' | 's' | 'u')) {
            (method, modifiers)
        } else {
            (rest, "")
        }
    } else {
        (rest, "")
    };
    if method_pattern.is_empty() {
        return Err(Error::InvalidQuery {
            query: query.to_owned(),
            message: "method pattern cannot be empty",
        });
    }

    let ignore_case = modifiers.contains('i');
    Ok(MethodQuery {
        class_pattern: normalize_case(class_pattern, ignore_case),
        method_pattern: normalize_case(method_pattern, ignore_case),
        include_signature: modifiers.contains('s'),
        ignore_case,
        skip_system_classes: modifiers.contains('u'),
    })
}

pub(crate) fn query_method_name(method: &JavaMethodMetadata, include_signature: bool) -> String {
    let name = if method.kind == MethodKind::Constructor {
        "$init"
    } else {
        &method.name
    };
    if include_signature {
        format!("{name}{}", method.signature)
    } else {
        name.to_owned()
    }
}

pub(crate) fn normalize_case(value: &str, ignore_case: bool) -> String {
    if ignore_case {
        value.to_ascii_lowercase()
    } else {
        value.to_owned()
    }
}

pub(crate) fn is_platform_class(name: &str) -> bool {
    name.starts_with("java.")
        || name.starts_with("javax.")
        || name.starts_with("android.")
        || name.starts_with("androidx.")
        || name.starts_with("dalvik.")
        || name.starts_with("com.android.")
}

pub(crate) fn glob_matches(pattern: &str, value: &str) -> bool {
    let pattern = pattern.as_bytes();
    let value = value.as_bytes();
    let (mut p, mut v) = (0, 0);
    let mut star = None;
    let mut star_value = 0;

    while v < value.len() {
        if p < pattern.len() && (pattern[p] == b'?' || pattern[p] == value[v]) {
            p += 1;
            v += 1;
        } else if p < pattern.len() && pattern[p] == b'*' {
            star = Some(p);
            star_value = v;
            p += 1;
        } else if let Some(star_index) = star {
            p = star_index + 1;
            star_value += 1;
            v = star_value;
        } else {
            return false;
        }
    }

    while p < pattern.len() && pattern[p] == b'*' {
        p += 1;
    }
    p == pattern.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{env::MethodKind, signature::MethodSignature};

    fn method(name: &str, kind: MethodKind, signature: &str) -> JavaMethodMetadata {
        JavaMethodMetadata {
            name: name.to_owned(),
            kind,
            signature: MethodSignature::parse(signature).unwrap(),
            modifiers: 0,
            id: std::ptr::dangling_mut(),
        }
    }

    #[test]
    fn parses_method_query_flags() {
        assert_eq!(
            parse_method_query("com.example.*!foo*/isu"),
            Ok(MethodQuery {
                class_pattern: "com.example.*".to_owned(),
                method_pattern: "foo*".to_owned(),
                include_signature: true,
                ignore_case: true,
                skip_system_classes: true,
            })
        );
    }

    #[test]
    fn rejects_method_queries_missing_required_parts() {
        assert_eq!(
            parse_method_query("com.example.*").unwrap_err(),
            Error::InvalidQuery {
                query: "com.example.*".to_owned(),
                message: "expected class!method query",
            }
        );
        assert_eq!(
            parse_method_query("!foo").unwrap_err(),
            Error::InvalidQuery {
                query: "!foo".to_owned(),
                message: "class pattern cannot be empty",
            }
        );
        assert_eq!(
            parse_method_query("com.example.*!").unwrap_err(),
            Error::InvalidQuery {
                query: "com.example.*!".to_owned(),
                message: "method pattern cannot be empty",
            }
        );
    }

    #[test]
    fn treats_unknown_query_suffix_as_part_of_method_pattern() {
        assert_eq!(
            parse_method_query("com.example.*!foo/bar"),
            Ok(MethodQuery {
                class_pattern: "com.example.*".to_owned(),
                method_pattern: "foo/bar".to_owned(),
                include_signature: false,
                ignore_case: false,
                skip_system_classes: false,
            })
        );
    }

    #[test]
    fn normalizes_case_when_query_is_case_insensitive() {
        assert_eq!(
            parse_method_query("Com.Example.*!Foo*/i"),
            Ok(MethodQuery {
                class_pattern: "com.example.*".to_owned(),
                method_pattern: "foo*".to_owned(),
                include_signature: false,
                ignore_case: true,
                skip_system_classes: false,
            })
        );
    }

    #[test]
    fn matches_simple_globs() {
        assert!(glob_matches("foo*", "foobar"));
        assert!(glob_matches("f?o", "foo"));
        assert!(!glob_matches("foo", "foobar"));
    }

    #[test]
    fn identifies_platform_classes_for_user_queries() {
        assert!(is_platform_class("java.lang.String"));
        assert!(is_platform_class("android.os.Process"));
        assert!(!is_platform_class("frida.java.bridge.rs.test.TestSubject"));
    }

    #[test]
    fn formats_query_constructor_names() {
        let method = method("<init>", MethodKind::Constructor, "(I)V");
        assert_eq!(query_method_name(&method, false), "$init");
        assert_eq!(query_method_name(&method, true), "$init(I)V");
    }
}
