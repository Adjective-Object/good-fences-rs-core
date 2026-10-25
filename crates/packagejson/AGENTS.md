# packagejson

Parse filesystem manifests from raw buffered bytes. Tracked reads seal exactly
the parsed bytes plus original identity/link observations, including misses;
never pair cached parsed values with current-on-disk hashes. Preserve ordered
export conditions and the documented bare-string exports extension.
Retain ordered recursive condition trees, including root conditional shorthand;
do not flatten nested conditions or treat objects as unsupported leaf targets.
Preserve recursive fallback arrays and distinguish invalid scalar targets from
null. Root array shorthand exports only ".", unlike the bare-string extension.
