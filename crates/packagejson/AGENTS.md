# packagejson

Parse filesystem manifests from raw buffered bytes. Tracked reads seal exactly
the parsed bytes plus original identity/link observations, including misses;
never pair cached parsed values with current-on-disk hashes. Preserve ordered
export conditions and the documented bare-string exports extension.
Retain ordered recursive condition trees, including root conditional shorthand;
do not flatten nested conditions or treat objects as unsupported leaf targets.
Preserve recursive fallback arrays and distinguish invalid scalar targets from
null. Root array shorthand exports only ".", unlike the bare-string extension.
Boolean main fields mean no legacy entry path; preserve string paths and exports
encapsulation, never interpret false as a stub or the filename "false".
Keep imports as explicit maps of recursive ordered targets; never normalize
imports using exports' bare-string wildcard extension.
