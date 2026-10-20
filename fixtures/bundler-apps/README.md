# Real browser application contracts

This corpus exercises real, pinned TodoMVC application **sources**, not upstream
prebundled JavaScript or a simulated DOM. It is separate from the Node-only
portable semantic fixtures under `fixtures/bundler`.

Install `agent-browser` and its browser using the official installation steps
at <https://agent-browser.dev/installation>. With `agent-browser` on `PATH`:

```sh
cargo build -p bundler --example build
node scripts/bundler-browser.mjs
```

`yarn test:bundler-browser` runs both commands. `AGENT_BROWSER_BIN` can name an
explicit agent-browser executable; `AGENT_BROWSER_EXECUTABLE_PATH` selects an
installed Chrome/Edge executable instead of downloading Chrome. The initial
Windows run used agent-browser 0.39.1 and installed Microsoft Edge.
After installing these prerequisites, execution needs no network access.

The runner verifies the entire SHA-256 inventory before execution, copies inputs
into managed `target/p1-test-tmp` directories, then independently exercises source
and public native build/publish output over loopback HTTP in isolated browser
sessions. It checks adding literal-text titles, individual completion, all three
hash routes, edit commit/cancel, clearing completed items, deletion, toggle-all,
and reload. It compares ordered titles, completion states, counters and routes.
Both pinned apps deliberately use in-memory storage: **reload resets the list**.
The ES6 app renders newest-first; the Web Components app renders oldest-first.
These are source contracts, not changes to normalize them to the TodoMVC spec.

Console errors, uncaught errors, unhandled rejections, script/stylesheet loading
errors, failed HTTP requests and unavailable diagnostics fail the run. Results
distinguish input, build, backend and output failures and continue to the next
app. Only native output can pass. Source behavior must pass before its bundle is
built. Browser controls use real input events; JavaScript evaluation only reads
state, including open shadow roots. CSS is extracted through the public build,
and HTML links emitted stylesheet URLs in application order.

`target/bundler-browser/results.json` and per-app `result.json` retain behavior
traces, console/page/network diagnostics, browser identity and build reports.
Source/output screenshots (or failure screenshots) live beside them.
Each successful build is copied into a fresh `bundle-*` directory before browser
execution, so exact output remains inspectable even when it fails. No source
modules are served alongside a bundle. Each run uses fresh browser sessions and
source/output origins; managed input/publication workspaces are removed in
`finally`. Browser sessions and HTTP servers close on success and failure.
Previous retained bundles are not overwritten or deleted.

`todomvc/upstream.json` records the full upstream revision, imported paths,
packaging adaptations and licensing. Vendored inputs are unchanged.
The app styling includes CC-BY-4.0 material by Sindre Sorhus, with its full license
and attribution retained; application and Nano ID MIT notices are also preserved.

Harness regression tests (no browser required):

```sh
node --test scripts/bundler-browser.test.mjs
```

This is an initial two-app smoke corpus, not full TodoMVC conformance, framework
coverage, performance measurement or cross-browser certification. It does not
claim support for JSX/TypeScript lowering or browser execution of Node/CommonJS
artifacts. The driver opts into native ESM CSS stubs; default caller-loaded CSS
URL wrappers currently require CommonJS.
