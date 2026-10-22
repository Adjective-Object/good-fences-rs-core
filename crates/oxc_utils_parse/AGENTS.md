# Parser policies

Keep `parse_file`'s unused-tracker compatibility policy separate from
`parse_bundler_file`. Bundler mode uses extension-sensitive language syntax and
an explicit package format; explicit ESM/CJS extensions override the package.
Static ESM syntax disambiguates CommonJS-default .js/.jsx/.ts/.tsx inputs at
parse time; dynamic import alone is not a marker. Reparse detected ESM under
module grammar. Explicit m/c extensions, including mjsx/mtsx/cjsx/ctsx, win;
reject ESM syntax in .cjs/.cjsx rather than reclassifying it.
Keep shared extension classification in bundler_source_type.
Never silently skip generated files.
Check semantic diagnostics after parsing before extracting executable records.
