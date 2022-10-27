# good-fences-rs-core

<!-- Core implementation of [`good-fences-rs`](https://github.com/Adjective-Object/good-fences.rs) -->


<!-- Written against `rustc 1.47.0 (18bf6b4f0 2020-10-07)` -->

A rust reimplementation of [good-fences](https://github.com/smikula/good-fences):
> Good-fences is a tool that allows you to segment a TypeScript project into conceptual areas and manage dependencies between those areas.
This is mostly a concern for large projects with many people working on them, where it is impossible for every developer to have a wholistic understanding of the entire codebase. JavaScript's module system is a specialized form of anarchy because any file can import any other file, possibly allowing access to code that was really meant to be an internal implementation detail of some larger system. Other languages have concepts like DLL boundaries and the internal keyword to mitigate this. Good-fences provides a way to enforce similar boundaries in the TypeScript world.

## Motivation

The original good-fences implementation came with some limitations:
- Its native dependencies only supported Node.js < v15.
- It had performance issues in some of our biggest projects (scanning 40k+ files).

Rust's safe concurrency and memory safety allows us to re-write original project with additional performance benefits, leaning on [swc](https://github.com/swc-project/swc/) for javascript/typescript parsing.
## Getting Started

`good-fences-rs` includes a CLI and an API, under the name `@good-fences/api`.

# Requirements

Compatible with `x86` and `x64` windows and linux platforms.

_Linux_:
- `GCLIB` >= 2.27 (preinstalled with ubuntu 18)
- Node.js > 14
- `npm`

# 

# Installing CLI

Via npm.

``` sh
npm install -g @good-fences/api
```

Cloing the repo:

``` sh
git clone https://github.com/Adjective-Object/good-fences-rs-core
cd good-fences-rs-core
npm install
npm run build
npm install -g
```

# Installing as Node dependency

``` sh
npm install @good-fences/api
```

Use it in your project:
``` js
import { goodFences } from '@good-fences/api';

goodFences({...});
```

# Using the CLI

To run `good-fences` cli we need at least two things:
- `fence.json` configuration files.
- A `tsconfig.json` file. (see [tsconfig reference](https://www.typescriptlang.org/tsconfig))

Let's assume a project like this:

```
├── my-project
│   ├── src
│   │   ├── **/*.ts
|   |   ├── index.js
|   │   ├── fence.json
|   tsconfig.json
```

From your terminal you can run this:
``` sh
cd my-project
good-fences src
```

## Arguments
---

- `[paths]`: the cli takes only the `paths` argument, a list, separated with spaces, of all directories that are going to be scanned.

## Options
---
### `--project` or `-p`

If you have your tsconfig file splitt and want to use the one containing `compilerOptions.paths` instead of the default `tsconfig.json`
``` sh
good-fences src --project tsconfig.with-paths.json
```
### `--baseUrl`

In cases like the one above, it could be that different tsconfig files have different `compilerOptions.baseUrl` configuration, you can override that valua from your specified `--project` file with `--baseUrl` flag.

``` sh
good-fences src --project tsconfig.without-baseurl.json --baseUrl .
```

### `--output` or `-o`
The `--output` flag takes a path. At the end of checking, fence violation errors will be saved to the provided path as json.

``` sh
good-fences src --output fenceViolations.json
cat fenceViolations.json
```

For some cases, scanning your `cwd` could be needed but most projects have `node_modules` that isn't necessary to perform evaluations, in those cases `--ignoreExternalFences` makes good-fences skip all directories and files from `node_modules`.
``` sh
good-fences . --ignoreExternalFences
```

### `--ignoredDirs`
This takes a list of regular expressions as input values, separated with spaces. In case certain directories need to be ignored during the fence evaluation, this will perform regular expression matching on fence paths to ignore them (e.g. `--ignoredDirs lib` will not evaluate files under any `lib` directory).

``` sh
good-fences src --ignoredDirs ignored1 ignored2 ...
```



