# Parser policies

Keep `parse_file`'s unused-tracker compatibility policy separate from
`parse_bundler_file`. Bundler mode uses extension-sensitive language syntax and
an explicit package format; explicit ESM/CJS extensions override the package.
Never infer package format from source content or silently skip generated files.
Check semantic diagnostics after parsing before extracting executable records.
Static ESM declarations in CommonJS-mode .js report the required package
"type":"module" or .mjs alternative; diagnostics never change the parser mode.
