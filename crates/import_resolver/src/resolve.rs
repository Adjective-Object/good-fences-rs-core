use std::path::{Path, PathBuf};

use compact_str::CompactString as CompactStr;
use serde::{Deserialize, Serialize};

/// The result of resolving an import specifier to a real file on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    pub path: PathBuf,
    pub slug: Option<CompactStr>,
}

/// A file-based import resolver. Both `base` and the returned `path` are real
/// filesystem paths — this trait never returns "virtual" or external-module
/// paths. Callers that need to distinguish node-builtin / unresolvable imports
/// should match on the error returned.
pub trait PathResolver: Send + Sync {
    fn resolve(&self, base: &Path, specifier: &str) -> anyhow::Result<Resolution>;
}

impl<T: PathResolver> PathResolver for &T {
    fn resolve(&self, base: &Path, specifier: &str) -> anyhow::Result<Resolution> {
        (*self).resolve(base, specifier)
    }
}

/// Target runtime environment used by the node-modules resolver to select
/// the appropriate `package.json` fields (`main` vs `browser`, etc.).
///
/// Copied verbatim from `swc_ecma_loader` so `import_resolver` no longer
/// depends on that crate.
#[derive(Debug, Copy, Clone, Serialize, Deserialize, Eq, PartialEq, Hash, Default)]
pub enum TargetEnv {
    #[serde(rename = "browser")]
    #[default]
    Browser,
    #[serde(rename = "node")]
    Node,
}

/// List of built-in Node.js package names (node@16 LTS).
///
/// Copied verbatim from `swc_ecma_loader` so `import_resolver` no longer
/// depends on that crate.
///
/// Run `node -p "require('module').builtinModules"` to regenerate.
pub const NODE_BUILTINS: &[&str] = &[
    "_http_agent",
    "_http_client",
    "_http_common",
    "_http_incoming",
    "_http_outgoing",
    "_http_server",
    "_stream_duplex",
    "_stream_passthrough",
    "_stream_readable",
    "_stream_transform",
    "_stream_wrap",
    "_stream_writable",
    "_tls_common",
    "_tls_wrap",
    "assert",
    "assert/strict",
    "async_hooks",
    "buffer",
    "child_process",
    "cluster",
    "console",
    "constants",
    "crypto",
    "dgram",
    "diagnostics_channel",
    "dns",
    "dns/promises",
    "domain",
    "events",
    "fs",
    "fs/promises",
    "http",
    "http2",
    "https",
    "inspector",
    "module",
    "net",
    "os",
    "path",
    "path/posix",
    "path/win32",
    "perf_hooks",
    "process",
    "punycode",
    "querystring",
    "readline",
    "readline/promises",
    "repl",
    "stream",
    "stream/consumers",
    "stream/promises",
    "stream/web",
    "string_decoder",
    "sys",
    "timers",
    "timers/promises",
    "tls",
    "trace_events",
    "tty",
    "url",
    "util",
    "util/types",
    "v8",
    "vm",
    "worker_threads",
    "zlib",
];
