# packagejson

Parse filesystem manifests from raw buffered bytes. Tracked reads seal exactly
the parsed bytes plus original identity/link observations, including misses;
never pair cached parsed values with current-on-disk hashes. Preserve ordered
export conditions and the documented bare-string exports extension.
